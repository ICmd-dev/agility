use std::{cell::RefCell, rc::Rc};

use crate::api::{LiftInto, WeakLiftInto};

thread_local! {
    static GLOBAL_BATCH_STATE: BatchState = BatchState::new();
}

struct BatchState(Rc<RefCell<BatchStateInner>>);

struct BatchStateInner {
    depth: usize,
    notification_depth: usize,
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
            notification_depth: 0,
        })))
    }

    fn enter(&self) {
        self.0.borrow_mut().depth += 1;
    }

    fn exit<T>(&self, signal: &SignalInner<T>) {
        let mut state = self.0.borrow_mut();
        state.depth = state.depth.saturating_sub(1);

        if state.depth == 0 {
            drop(state);

            // Notify the primary signal that triggered the batch
            if *signal.needs_notification.borrow() {
                *signal.needs_notification.borrow_mut() = false;
                signal.notify_subscribers();
            }
        }
    }

    fn is_notifying(&self) -> bool {
        self.0.borrow().notification_depth > 0
    }

    fn enter_notification(&self) {
        self.0.borrow_mut().notification_depth += 1;
    }

    fn exit_notification(&self) {
        let mut state = self.0.borrow_mut();
        state.notification_depth = state.notification_depth.saturating_sub(1);
    }
}

pub struct SignalInner<'a, T> {
    pub value: RefCell<T>,
    pub subscribers: RefCell<Vec<Box<dyn Fn() -> bool + 'a>>>,
    pub needs_notification: RefCell<bool>,
    pub version: RefCell<u64>,
}

pub struct Signal<'a, T>(pub Rc<SignalInner<'a, T>>);

pub struct BatchGuard<'s, 'a, T>(&'s Signal<'a, T>);

impl<T> Drop for BatchGuard<'_, '_, T> {
    fn drop(&mut self) {
        GLOBAL_BATCH_STATE.with(|bs| bs.exit(&self.0.0));
    }
}

impl<'a, T: Clone> Clone for Signal<'a, T> {
    fn clone(&self) -> Self {
        Signal(Rc::new(SignalInner {
            value: RefCell::new(self.0.value.borrow().clone()),
            subscribers: RefCell::new(Vec::new()),
            needs_notification: RefCell::new(false),
            version: RefCell::new(0),
        }))
    }
}

impl<'a, T> Signal<'a, T> {
    pub fn new(initial_value: T) -> Signal<'a, T> {
        Signal(Rc::new(SignalInner {
            value: RefCell::new(initial_value),
            subscribers: RefCell::new(Vec::new()),
            needs_notification: RefCell::new(false),
            version: RefCell::new(0),
        }))
    }

    pub fn send(&self, value: T) -> BatchGuard<'_, 'a, T> {
        GLOBAL_BATCH_STATE.with(|bs| bs.enter());
        *self.0.value.borrow_mut() = value;
        *self.0.version.borrow_mut() += 1;
        *self.0.needs_notification.borrow_mut() = true;

        BatchGuard(self)
    }

    pub fn send_with<F>(&self, f: F) -> BatchGuard<'_, 'a, T>
    where
        F: FnOnce(&mut T),
    {
        GLOBAL_BATCH_STATE.with(|bs| bs.enter());
        f(&mut self.0.value.borrow_mut());
        *self.0.version.borrow_mut() += 1;
        *self.0.needs_notification.borrow_mut() = true;

        BatchGuard(self)
    }

    pub fn send_now(&self, value: T) {
        *self.0.value.borrow_mut() = value;
        *self.0.version.borrow_mut() += 1;
        self.0.notify_subscribers();
    }

    pub fn send_with_now<F>(&self, f: F)
    where
        F: FnOnce(&mut T),
    {
        f(&mut self.0.value.borrow_mut());
        *self.0.version.borrow_mut() += 1;
        self.0.notify_subscribers();
    }

    fn send_deferred_with<F>(&self, f: F)
    where
        F: FnOnce(&mut T),
    {
        f(&mut self.0.value.borrow_mut());
        *self.0.version.borrow_mut() += 1;

        GLOBAL_BATCH_STATE.with(|bs| {
            if bs.is_notifying() {
                let was_marked = *self.0.needs_notification.borrow();
                *self.0.needs_notification.borrow_mut() = true;
                if was_marked {
                    *self.0.needs_notification.borrow_mut() = false;
                    self.0.notify_subscribers();
                }
            } else {
                self.0.notify_subscribers();
            }
        });
    }

    #[doc(hidden)]
    pub fn __send_deferred_with<F>(&self, f: F)
    where
        F: FnOnce(&mut T),
    {
        self.send_deferred_with(f);
    }
    pub fn map<F, U>(&self, f: F) -> Signal<'a, U>
    where
        F: Fn(&T) -> U + 'a,
        U: 'a,
        T: 'a,
    {
        // Default map uses strong Rc references to keep signals alive.
        let new_signal = Signal::new(f(&self.0.value.borrow()));
        let new_signal_clone = Signal(Rc::clone(&new_signal.0));
        let self_clone = Signal(Rc::clone(&self.0));
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

    pub fn clone_refered(&self) -> Signal<'a, T> {
        Signal(Rc::clone(&self.0))
    }
}

impl<'a, T> SignalInner<'a, T> {
    fn notify_subscribers(&self) {
        GLOBAL_BATCH_STATE.with(|bs| {
            bs.enter_notification();
        });

        let mut subscribers = self.subscribers.borrow_mut();
        subscribers.retain(|subscriber| subscriber());
        drop(subscribers);

        GLOBAL_BATCH_STATE.with(|bs| {
            bs.exit_notification();
        });
    }
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
    use crate::api::RcRef;

    let new_combined = Signal::new((
        left.0.value.borrow().clone(),
        right.0.value.borrow().clone(),
    ));

    let create_subscription =
        |left: &Signal<'a, T>, right: &Signal<'a, U>, combined: &Signal<'a, (T, U)>| {
            let ref_combined = RcRef::new(Rc::clone(&combined.0), use_weak);
            let ref_left = RcRef::new(Rc::clone(&left.0), use_weak);
            let ref_right = RcRef::new(Rc::clone(&right.0), use_weak);

            let last_left_version = RefCell::new(0u64);
            let last_right_version = RefCell::new(0u64);

            Box::new(move || {
                ref_combined
                    .upgrade()
                    .zip(ref_left.upgrade())
                    .zip(ref_right.upgrade())
                    .map(|((combined_inner, left_inner), right_inner)| {
                        let left_ver = *left_inner.version.borrow();
                        let right_ver = *right_inner.version.borrow();
                        let left_changed = left_ver != *last_left_version.borrow();
                        let right_changed = right_ver != *last_right_version.borrow();

                        if left_changed || right_changed {
                            *last_left_version.borrow_mut() = left_ver;
                            *last_right_version.borrow_mut() = right_ver;

                            let combined = Signal(combined_inner);
                            combined.send_deferred_with(|tuple| {
                                if left_changed {
                                    tuple.0 = left_inner.value.borrow().clone();
                                }
                                if right_changed {
                                    tuple.1 = right_inner.value.borrow().clone();
                                }
                            });
                        }
                        true
                    })
                    .unwrap_or(false)
            }) as Box<dyn Fn() -> bool + 'a>
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
    use crate::api::RcRef;

    let initial_values: Vec<T> = signals.iter().map(|s| s.0.value.borrow().clone()).collect();
    let new_sequence = Signal::new(initial_values);

    let create_subscription = |signals: &[Signal<'a, T>], sequence: &Signal<'a, Vec<T>>| {
        let ref_sequence = RcRef::new(Rc::clone(&sequence.0), use_weak);
        let ref_signals: Vec<_> = signals
            .iter()
            .map(|s| RcRef::new(Rc::clone(&s.0), use_weak))
            .collect();

        let last_versions = RefCell::new(vec![0u64; signals.len()]);

        Box::new(move || {
            ref_sequence
                .upgrade()
                .and_then(|sequence_inner| {
                    let upgraded: Option<Vec<_>> =
                        ref_signals.iter().map(|rs| rs.upgrade()).collect();

                    upgraded.map(|signal_inners| {
                        let versions: Vec<u64> = signal_inners
                            .iter()
                            .map(|si| *si.version.borrow())
                            .collect();

                        let last = last_versions.borrow();
                        let changed_indices: Vec<_> = versions
                            .iter()
                            .enumerate()
                            .filter_map(|(i, &v)| (v != last[i]).then_some(i))
                            .collect();

                        if !changed_indices.is_empty() {
                            drop(last);
                            *last_versions.borrow_mut() = versions;

                            let sequence = Signal(sequence_inner);
                            sequence.send_deferred_with(|vec| {
                                for &i in &changed_indices {
                                    vec[i] = signal_inners[i].value.borrow().clone();
                                }
                            });
                        }
                        true
                    })
                })
                .unwrap_or(false)
        }) as Box<dyn Fn() -> bool + 'a>
    };

    for signal in signals {
        signal
            .0
            .subscribers
            .borrow_mut()
            .push(create_subscription(signals, &new_sequence));
    }

    new_sequence
}
