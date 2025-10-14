use std::{cell::RefCell, rc::Rc};

thread_local! {
    static GLOBAL_BATCH_STATE: BatchState = BatchState::new();
}

pub fn add(left: u64, right: u64) -> u64 {
    left + right
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let signal = Signal::new(5);
        let signal_r = signal.map(|x| {
            let new = x + 1;
            println!("signal_r: {}", new);
            new
        });
        let signal_l = signal.map(|x| {
            let new = x * 2;
            println!("signal_l: {}", new);
            new
        });
        (signal_r, signal_l)
            .lift()
            .map(|(r, l)| println!("({}, {})", r, l));
        println!("Sending 7...");
        signal.send(7);
    }

    #[test]
    fn test_sequence() {
        let s1 = Signal::new(1);
        let s2 = Signal::new(2);
        let s3 = Signal::new(3);

        let seq = [s1.clone(), s2.clone(), s3.clone()].lift();
        seq.map(|v| {
            println!("Sequence: {:?}", v);
        });

        println!("Updating individually...");
        s1.send(10);
        s2.send(20);
        s3.send(30);

        println!("Batching updates...");
        (
            s1.send(100),
            s2.send(200),
            s3.send(300),
            println!("All sends called, waiting for drop..."),
        );
        println!("Batch complete!");
    }

    #[test]
    fn test_batch() {
        let signal = Signal::new(0);
        let _mapped = signal.map(|x| {
            println!("Mapped value: {}", x);
            *x
        });

        println!("Non-batched sends (each triggers immediately):");
        signal.send(1);
        signal.send(2);

        println!("\nBatched sends (triggers once after scope):");
        {
            let _g1 = signal.send(10);
            let _g2 = signal.send(20);
            println!("Inside batch scope - no notification yet");
        }
        println!("Batch complete - notification fired!\n");
    }

    #[test]
    fn test_batch_combine() {
        let x = Signal::new(1);
        let y = Signal::new(2);

        let combined = x.combine(&y);
        combined.map(|(a, b)| {
            println!("Combined: ({}, {})", a, b);
        });

        println!("Non-batched - triggers twice:");
        x.send(10);
        y.send(20);

        println!("\nBatched - triggers once:");
        {
            let _gx = x.send(100);
            let _gy = y.send(200);
        }
        println!("Done!");
    }
}

struct BatchState(Rc<RefCell<BatchStateInner>>);

struct BatchStateInner {
    depth: usize,
    notified_signals: Vec<*const ()>,
}

impl Clone for BatchState {
    fn clone(&self) -> Self {
        BatchState(Rc::clone(&self.0))
    }
}

impl BatchState {
    fn new() -> Self {
        BatchState(Rc::new(RefCell::new(BatchStateInner {
            depth: 0,
            notified_signals: Vec::new(),
        })))
    }

    fn enter(&self) {
        self.0.borrow_mut().depth += 1;
    }

    fn exit<T>(&self, signal: &SignalInner<T>) {
        let mut state = self.0.borrow_mut();
        state.depth = state.depth.saturating_sub(1);

        if state.depth == 0 {
            state.notified_signals.clear();
            drop(state);

            if *signal.needs_notification.borrow() {
                *signal.needs_notification.borrow_mut() = false;
                signal.notify_subscribers();
            }
        }
    }
}

struct SignalInner<'a, T> {
    value: RefCell<T>,
    subscribers: RefCell<Vec<Box<dyn Fn() -> bool + 'a>>>,
    needs_notification: RefCell<bool>,
}

pub struct Signal<'a, T>(Rc<SignalInner<'a, T>>);

pub struct BatchGuard<'a, T>(Signal<'a, T>);

impl<'a, T> Drop for BatchGuard<'a, T> {
    fn drop(&mut self) {
        GLOBAL_BATCH_STATE.with(|bs| bs.exit(&self.0.0));
    }
}

impl<'a, T> Clone for Signal<'a, T> {
    fn clone(&self) -> Self {
        Signal(Rc::clone(&self.0))
    }
}

impl<'a, T> Signal<'a, T> {
    pub fn new(initial_value: T) -> Signal<'a, T> {
        Signal(Rc::new(SignalInner {
            value: RefCell::new(initial_value),
            subscribers: RefCell::new(Vec::new()),
            needs_notification: RefCell::new(false),
        }))
    }

    pub fn send(&self, value: T) -> BatchGuard<'a, T> {
        GLOBAL_BATCH_STATE.with(|bs| bs.enter());
        *self.0.value.borrow_mut() = value;
        *self.0.needs_notification.borrow_mut() = true;

        BatchGuard(self.clone())
    }

    pub fn send_with<F>(&self, f: F) -> BatchGuard<'a, T>
    where
        F: FnOnce(&mut T),
    {
        GLOBAL_BATCH_STATE.with(|bs| bs.enter());
        f(&mut self.0.value.borrow_mut());
        *self.0.needs_notification.borrow_mut() = true;

        BatchGuard(self.clone())
    }

    pub fn send_now(&self, value: T) {
        *self.0.value.borrow_mut() = value;
        self.0.notify_subscribers();
    }

    pub fn send_with_now<F>(&self, f: F)
    where
        F: FnOnce(&mut T),
    {
        f(&mut self.0.value.borrow_mut());
        self.0.notify_subscribers();
    }
    pub fn map<F, U>(&self, f: F) -> Signal<'a, U>
    where
        F: Fn(&T) -> U + 'a,
        U: 'a,
        T: 'a,
    {
        // Default map uses strong Rc references to keep signals alive.
        let new_signal = Signal::new(f(&self.0.value.borrow()));
        let new_signal_clone = new_signal.clone();
        let self_clone = self.clone();
        let subscription = Box::new(move || {
            let new_value = f(&self_clone.0.value.borrow());
            new_signal_clone.send_now(new_value);
            true
        });
        self.0.subscribers.borrow_mut().push(subscription);
        new_signal
    }

    /// Map using weak references: if either signal is dropped, subscription is removed.
    pub fn weak_map<F, U>(&self, f: F) -> Signal<'a, U>
    where
        F: Fn(&T) -> U + 'a,
        U: 'a,
        T: 'a,
    {
        // Original behavior captured weak references to avoid keeping signals alive.
        let new_signal = Signal::new(f(&self.0.value.borrow()));
        let weak_new_signal = Rc::downgrade(&new_signal.0);
        let weak_self = Rc::downgrade(&self.0);
        let subscription = Box::new(move || {
            weak_new_signal
                .upgrade()
                .and_then(|new_signal_inner| {
                    weak_self
                        .upgrade()
                        .map(|self_inner| (Signal(new_signal_inner), Signal(self_inner)))
                })
                .map(|(new_signal, self_signal)| {
                    let new_value = f(&self_signal.0.value.borrow());
                    new_signal.send_now(new_value);
                    true
                })
                .unwrap_or(false)
        });
        self.0.subscribers.borrow_mut().push(subscription);
        new_signal
    }
    pub fn combine<U>(&self, other: &Signal<'a, U>) -> Signal<'a, (T, U)>
    where
        U: Clone + 'a,
        T: Clone + 'a,
    {
        combine_impl(self, other, false)
    }

    pub fn weak_combine<U>(&self, other: &Signal<'a, U>) -> Signal<'a, (T, U)>
    where
        U: Clone + 'a,
        T: Clone + 'a,
    {
        combine_impl(self, other, true)
    }

    pub fn sequence(signals: &[Signal<'a, T>]) -> Signal<'a, Vec<T>>
    where
        T: Clone + 'a,
    {
        sequence_impl(signals, false)
    }

    pub fn weak_sequence(signals: &[Signal<'a, T>]) -> Signal<'a, Vec<T>>
    where
        T: Clone + 'a,
    {
        sequence_impl(signals, true)
    }
}

impl<'a, T> SignalInner<'a, T> {
    fn notify_subscribers(&self) {
        let mut subscribers = self.subscribers.borrow_mut();
        subscribers.retain(|subscriber| subscriber());
    }
}

pub trait LiftInto<T> {
    fn lift(self) -> T;
}

pub trait WeakLiftInto<T> {
    fn weak_lift(self) -> T;
}

impl<'a, T, U> LiftInto<Signal<'a, (T, U)>> for (Signal<'a, T>, Signal<'a, U>)
where
    T: Clone + 'a,
    U: Clone + 'a,
{
    fn lift(self) -> Signal<'a, (T, U)> {
        self.0.combine(&self.1)
    }
}

impl<'a, T, U> WeakLiftInto<Signal<'a, (T, U)>> for (Signal<'a, T>, Signal<'a, U>)
where
    T: Clone + 'a,
    U: Clone + 'a,
{
    fn weak_lift(self) -> Signal<'a, (T, U)> {
        self.0.weak_combine(&self.1)
    }
}

impl<'a, T> LiftInto<Signal<'a, Vec<T>>> for &[Signal<'a, T>]
where
    T: Clone + 'a,
{
    fn lift(self) -> Signal<'a, Vec<T>> {
        Signal::sequence(self)
    }
}

impl<'a, T> LiftInto<Signal<'a, Vec<T>>> for Vec<Signal<'a, T>>
where
    T: Clone + 'a,
{
    fn lift(self) -> Signal<'a, Vec<T>> {
        Signal::sequence(&self)
    }
}

impl<'a, const N: usize, T> LiftInto<Signal<'a, Vec<T>>> for [Signal<'a, T>; N]
where
    T: Clone + 'a,
{
    fn lift(self) -> Signal<'a, Vec<T>> {
        Signal::sequence(&self)
    }
}

impl<'a, T> WeakLiftInto<Signal<'a, Vec<T>>> for &[Signal<'a, T>]
where
    T: Clone + 'a,
{
    fn weak_lift(self) -> Signal<'a, Vec<T>> {
        Signal::weak_sequence(self)
    }
}

impl<'a, T> WeakLiftInto<Signal<'a, Vec<T>>> for Vec<Signal<'a, T>>
where
    T: Clone + 'a,
{
    fn weak_lift(self) -> Signal<'a, Vec<T>> {
        Signal::weak_sequence(&self)
    }
}

impl<'a, const N: usize, T> WeakLiftInto<Signal<'a, Vec<T>>> for [Signal<'a, T>; N]
where
    T: Clone + 'a,
{
    fn weak_lift(self) -> Signal<'a, Vec<T>> {
        Signal::weak_sequence(&self)
    }
}

fn combine_impl<'a, T, U>(
    left: &Signal<'a, T>,
    right: &Signal<'a, U>,
    use_weak: bool,
) -> Signal<'a, (T, U)>
where
    U: Clone + 'a,
    T: Clone + 'a,
{
    let new_combined = Signal::new((
        left.0.value.borrow().clone(),
        right.0.value.borrow().clone(),
    ));

    let create_subscription =
        |signal: &Signal<'a, T>, right: &Signal<'a, U>, combined: &Signal<'a, (T, U)>| {
            if use_weak {
                let weak_combined = Rc::downgrade(&combined.0);
                let weak_signal = Rc::downgrade(&signal.0);
                let weak_right = Rc::downgrade(&right.0);
                Box::new(move || {
                    weak_combined
                        .upgrade()
                        .zip(weak_signal.upgrade())
                        .zip(weak_right.upgrade())
                        .map(|((combined_inner, signal_inner), right_inner)| {
                            let combined = Signal(combined_inner);
                            let new_value = (
                                signal_inner.value.borrow().clone(),
                                right_inner.value.borrow().clone(),
                            );
                            combined.send_now(new_value);
                            true
                        })
                        .unwrap_or(false)
                }) as Box<dyn Fn() -> bool + 'a>
            } else {
                let combined = combined.clone();
                let signal = signal.clone();
                let right = right.clone();
                Box::new(move || {
                    let new_value = (
                        signal.0.value.borrow().clone(),
                        right.0.value.borrow().clone(),
                    );
                    combined.send_now(new_value);
                    true
                }) as Box<dyn Fn() -> bool + 'a>
            }
        };

    left.0
        .subscribers
        .borrow_mut()
        .push(create_subscription(left, right, &new_combined));
    right
        .0
        .subscribers
        .borrow_mut()
        .push(create_subscription(left, right, &new_combined));

    new_combined
}

fn sequence_impl<'a, T>(signals: &[Signal<'a, T>], use_weak: bool) -> Signal<'a, Vec<T>>
where
    T: Clone + 'a,
{
    let initial_values: Vec<T> = signals.iter().map(|s| s.0.value.borrow().clone()).collect();

    let sequenced = Signal::new(initial_values);

    for signal in signals.iter() {
        let subscription = if use_weak {
            let weak_sequenced = Rc::downgrade(&sequenced.0);
            let weak_signals: Vec<_> = signals.iter().map(|s| Rc::downgrade(&s.0)).collect();
            Box::new(move || {
                weak_sequenced
                    .upgrade()
                    .map(|sequenced_inner| {
                        let sequenced = Signal(sequenced_inner);
                        let new_values: Option<Vec<T>> = weak_signals
                            .iter()
                            .map(|weak| weak.upgrade().map(|inner| inner.value.borrow().clone()))
                            .collect();

                        if let Some(values) = new_values {
                            sequenced.send_now(values);
                        }
                        true
                    })
                    .unwrap_or(false)
            }) as Box<dyn Fn() -> bool + 'a>
        } else {
            let sequenced = sequenced.clone();
            let signals_clone: Vec<_> = signals.iter().cloned().collect();
            Box::new(move || {
                let new_values: Vec<T> = signals_clone
                    .iter()
                    .map(|s| s.0.value.borrow().clone())
                    .collect();
                sequenced.send_now(new_values);
                true
            }) as Box<dyn Fn() -> bool + 'a>
        };

        signal.0.subscribers.borrow_mut().push(subscription);
    }

    sequenced
}
