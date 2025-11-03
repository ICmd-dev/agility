use std::{cell::RefCell, iter, rc::Rc};

use crate::api::Liftable;

pub trait SignalExt<'a> {
    fn react(&self);
    fn guard(&self) -> SignalGuard<'a>;
    fn decrease_dirty(&self);
    fn get_dirty(&self) -> isize;
    fn clone_box(&self) -> Box<dyn SignalExt<'a> + 'a>;
    fn collect_guards_recursive(&self, result: &mut Vec<SignalGuardInner<'a>>);
    fn collect_predecessors_recursive(&self, result: &mut Vec<SignalGuardInner<'a>>);
    fn reset_explicitly_modified(&self);
}

// Helper struct to hold weak references that can be upgraded
pub struct WeakSignalRef<'a> {
    upgrade: Box<dyn Fn() -> Option<Box<dyn SignalExt<'a> + 'a>> + 'a>,
}

impl<'a> WeakSignalRef<'a> {
    pub fn new<T: 'a>(signal: &Signal<'a, T>) -> Self {
        let weak = Rc::downgrade(&signal.0);
        WeakSignalRef {
            upgrade: Box::new(move || {
                weak.upgrade()
                    .map(|rc| Box::new(Signal(rc)) as Box<dyn SignalExt<'a> + 'a>)
            }),
        }
    }

    pub fn upgrade(&self) -> Option<Box<dyn SignalExt<'a> + 'a>> {
        (self.upgrade)()
    }

    pub fn is_alive(&self) -> bool {
        self.upgrade().is_some()
    }
}

pub struct SignalGuardInner<'a>(Box<dyn SignalExt<'a> + 'a>);

#[allow(dead_code)]
#[allow(unused_must_use)]
pub struct SignalGuard<'a>(Vec<SignalGuardInner<'a>>);

impl<'a> Drop for SignalGuardInner<'a> {
    fn drop(&mut self) {
        self.0.decrease_dirty();
        if self.0.get_dirty() == 0 {
            self.0.react();
            self.0.reset_explicitly_modified();
        }
    }
}

impl<'a> Drop for SignalGuard<'a> {
    fn drop(&mut self) {
        // First drop all inner guards (triggers immediate reactions)
        drop(std::mem::take(&mut self.0));
    }
}

pub struct SignalInner<'a, T> {
    pub(crate) value: RefCell<T>,
    pub(crate) react_fns: RefCell<Vec<Box<dyn Fn() + 'a>>>,
    pub(crate) successors: RefCell<Vec<WeakSignalRef<'a>>>,
    pub(crate) predecessors: RefCell<Vec<WeakSignalRef<'a>>>,
    pub(crate) dirty: RefCell<isize>,
    pub(crate) explicitly_modified: RefCell<bool>,
}

pub struct Signal<'a, T>(pub(crate) Rc<SignalInner<'a, T>>);

impl<'a, T: 'a> Signal<'a, T> {
    pub fn new(initial: T) -> Self {
        let inner = Rc::new(SignalInner {
            value: RefCell::new(initial),
            react_fns: RefCell::new(Vec::new()),
            successors: RefCell::new(Vec::new()),
            predecessors: RefCell::new(Vec::new()),
            dirty: RefCell::new(0),
            explicitly_modified: RefCell::new(false),
        });
        Signal(inner)
    }

    pub fn send(&self, new_value: T) -> SignalGuard<'a> {
        self.modify(|v| *v = new_value);
        *self.0.explicitly_modified.borrow_mut() = true;
        self.guard()
    }

    pub fn send_with<F>(&self, f: F) -> SignalGuard<'a>
    where
        F: FnOnce(&mut T),
    {
        self.modify(f);
        self.guard()
    }

    pub fn map<U: 'a, F>(&self, f: F) -> Signal<'a, U>
    where
        F: Fn(&T) -> U + 'a,
    {
        let new_signal = Signal::new(f(&self.0.value.borrow()));
        let result_new_signal = new_signal.clone();
        let new_signal_for_react = Rc::downgrade(&new_signal.0);
        let source_for_closure = Rc::downgrade(&self.0);

        let react_fn = Box::new(move || {
            // Only update if the signal wasn't explicitly modified
            if let Some(new_sig) = new_signal_for_react.upgrade() {
                if !*new_sig.explicitly_modified.borrow() {
                    if let Some(source) = source_for_closure.upgrade() {
                        let new_value = f(&source.value.borrow());
                        *new_sig.value.borrow_mut() = new_value;
                    }
                }
            }
        });

        self.0.react_fns.borrow_mut().push(react_fn);
        self.0
            .successors
            .borrow_mut()
            .push(WeakSignalRef::new(&new_signal));
        result_new_signal
    }

    pub fn comap<F, U>(&self, f: F) -> Signal<'a, U>
    where
        F: Fn(&U) -> T + 'a,
        U: Default + 'a,
    {
        let new_signal = Signal::new(U::default());
        let result_new_signal = new_signal.clone();
        let source_inner = Rc::downgrade(&self.0);
        let new_signal_rc = Rc::downgrade(&new_signal.0);

        let react_fn = Box::new(move || {
            if let Some(new_sig) = new_signal_rc.upgrade() {
                if *new_sig.explicitly_modified.borrow() {
                    let u_value_ref = new_sig.value.borrow();
                    let t_value = f(&u_value_ref);
                    drop(u_value_ref);

                    if let Some(source) = source_inner.upgrade() {
                        *source.value.borrow_mut() = t_value;
                        *source.explicitly_modified.borrow_mut() = true;
                    }
                }
            }
        });
        new_signal.0.react_fns.borrow_mut().push(react_fn);

        // Register self (source/result) as a predecessor of new_signal (backward dependency)
        // When new_signal changes, it propagates backward to self
        new_signal
            .0
            .predecessors
            .borrow_mut()
            .push(WeakSignalRef::new(self));

        result_new_signal
    }

    pub fn promap<F, G, U>(&self, f: F, g: G) -> Signal<'a, U>
    where
        F: Fn(&T) -> U + 'a,
        G: Fn(&U) -> T + 'a,
        U: Default + 'a,
    {
        let new_signal = Signal::new(U::default());
        let result_new_signal = new_signal.clone();
        let source_weak = Rc::downgrade(&self.0);
        let new_signal_weak = Rc::downgrade(&new_signal.0);

        // Forward reaction: T -> U (covariant)
        let source_inner = source_weak.clone();
        let new_signal_rc = new_signal_weak.clone();
        let forward_react_fn = Box::new(move || {
            if let Some(new_sig) = new_signal_rc.upgrade() {
                if !*new_sig.explicitly_modified.borrow() {
                    if let Some(source) = source_inner.upgrade() {
                        let t_value = source.value.borrow();
                        let u_value = f(&t_value);
                        drop(t_value);
                        *new_sig.value.borrow_mut() = u_value;
                    }
                }
            }
        });

        self.0.react_fns.borrow_mut().push(forward_react_fn);
        self.0
            .successors
            .borrow_mut()
            .push(WeakSignalRef::new(&new_signal));

        // Backward reaction: U -> T (contravariant)
        let new_signal_rc_back = new_signal_weak.clone();
        let source_inner_back = source_weak.clone();

        let backward_react_fn = Box::new(move || {
            if let Some(new_sig) = new_signal_rc_back.upgrade() {
                if *new_sig.explicitly_modified.borrow() {
                    let u_value_ref = new_sig.value.borrow();
                    let t_value = g(&u_value_ref);
                    drop(u_value_ref);

                    if let Some(source) = source_inner_back.upgrade() {
                        *source.value.borrow_mut() = t_value;
                        *source.explicitly_modified.borrow_mut() = true;
                    }
                }
            }
        });
        new_signal.0.react_fns.borrow_mut().push(backward_react_fn);

        // Register self (source) as a predecessor of new_signal (backward dependency)
        // When new_signal changes, it propagates backward to self
        new_signal
            .0
            .predecessors
            .borrow_mut()
            .push(WeakSignalRef::new(self));

        result_new_signal
    }

    pub fn with<S>(&self, another: S) -> Signal<'a, (T, S::Inner)>
    where
        S: Liftable<'a>,
        S::Inner: Clone + 'a,
        T: Clone,
    {
        let another = another.as_ref();
        let new_signal = Signal::new((
            self.0.value.borrow().clone(),
            another.0.value.borrow().clone(),
        ));
        let result_new_signal = new_signal.clone();
        let new_signal_weak = Rc::downgrade(&new_signal.0);
        let source_for_closure_self = Rc::downgrade(&self.0);
        let source_for_closure_another = Rc::downgrade(&another.0);

        let react_fn_self = Box::new(move || {
            if let Some(new_sig) = new_signal_weak.upgrade() {
                if !*new_sig.explicitly_modified.borrow() {
                    if let Some(source) = source_for_closure_self.upgrade() {
                        new_sig.value.borrow_mut().0 = source.value.borrow().clone();
                    }
                }
            }
        });
        let new_signal_weak_2 = Rc::downgrade(&new_signal.0);
        let react_fn_another = Box::new(move || {
            if let Some(new_sig) = new_signal_weak_2.upgrade() {
                if !*new_sig.explicitly_modified.borrow() {
                    if let Some(source) = source_for_closure_another.upgrade() {
                        new_sig.value.borrow_mut().1 = source.value.borrow().clone();
                    }
                }
            }
        });
        self.0.react_fns.borrow_mut().push(react_fn_self);
        another.0.react_fns.borrow_mut().push(react_fn_another);
        self.0
            .successors
            .borrow_mut()
            .push(WeakSignalRef::new(&result_new_signal));
        another
            .0
            .successors
            .borrow_mut()
            .push(WeakSignalRef::new(&result_new_signal));
        result_new_signal
    }

    pub fn extend<S>(&self, others: impl IntoIterator<Item = S>) -> Signal<'a, Vec<T>>
    where
        S: Liftable<'a, Inner = T>,
        T: Clone,
    {
        // Collect the iterator into owned Signal clones so we don't keep
        // references to temporary `S` values that would be dropped.
        let others_signals: Vec<Signal<'a, T>> =
            others.into_iter().map(|s| s.as_ref().clone()).collect();

        let new_signal: Signal<'a, Vec<T>> = Signal::new(
            iter::once(self)
                .chain(others_signals.iter())
                .map(|s| s.0.value.borrow().clone())
                .collect(),
        );
        let result_new_signal = new_signal.clone();

        iter::once(self)
            .chain(others_signals.iter())
            .enumerate()
            .for_each(|(index, signal)| {
                let new_signal_weak = Rc::downgrade(&new_signal.0);
                let source_for_closure = Rc::downgrade(&signal.0);

                let react_fn = Box::new(move || {
                    if let Some(new_sig) = new_signal_weak.upgrade() {
                        if !*new_sig.explicitly_modified.borrow() {
                            if let Some(source) = source_for_closure.upgrade() {
                                new_sig.value.borrow_mut()[index] = source.value.borrow().clone();
                            }
                        }
                    }
                });

                signal.0.react_fns.borrow_mut().push(react_fn);
                signal
                    .0
                    .successors
                    .borrow_mut()
                    .push(WeakSignalRef::new(&new_signal));
            });

        result_new_signal
    }

    pub(crate) fn modify(&self, f: impl FnOnce(&mut T)) {
        let mut value = self.0.value.borrow_mut();
        f(&mut value);
    }

    fn mark_dirty(&self) {
        *self.0.dirty.borrow_mut() += 1;
    }

    fn collect_and_iterate<F>(&self, refs: &RefCell<Vec<WeakSignalRef<'a>>>, mut callback: F)
    where
        F: FnMut(&dyn SignalExt<'a>),
    {
        refs.borrow_mut().retain(|s| s.is_alive());
        for s in refs.borrow().iter() {
            if let Some(signal) = s.upgrade() {
                callback(&*signal);
            }
        }
    }

    fn collect_guards(&self, result: &mut Vec<SignalGuardInner<'a>>) {
        self.mark_dirty();
        result.push(SignalGuardInner(self.clone_box()));
        self.collect_and_iterate(&self.0.successors, |signal| {
            signal.collect_guards_recursive(result);
        });
        self.collect_and_iterate(&self.0.predecessors, |signal| {
            signal.collect_predecessors_recursive(result);
        });
    }

    pub fn lift_from_array<S, const N: usize>(items: [S; N]) -> Signal<'a, [S::Inner; N]>
    where
        S: Liftable<'a>,
        S::Inner: Clone + 'a,
    {
        let signals: [Signal<'a, S::Inner>; N] = std::array::from_fn(|i| items[i].as_ref().clone());

        let initial: [S::Inner; N] = std::array::from_fn(|i| signals[i].0.value.borrow().clone());

        let new_signal: Signal<'a, [S::Inner; N]> = Signal::new(initial);
        let result_new_signal = new_signal.clone();

        for (index, signal) in signals.iter().enumerate() {
            let new_signal_weak = Rc::downgrade(&new_signal.0);
            let source_for_closure = Rc::downgrade(&signal.0);

            let react_fn = Box::new(move || {
                if let Some(new_sig) = new_signal_weak.upgrade() {
                    if !*new_sig.explicitly_modified.borrow() {
                        if let Some(source) = source_for_closure.upgrade() {
                            new_sig.value.borrow_mut()[index] = source.value.borrow().clone();
                        }
                    }
                }
            });

            signal.0.react_fns.borrow_mut().push(react_fn);
            signal
                .0
                .successors
                .borrow_mut()
                .push(WeakSignalRef::new(&new_signal));
        }

        result_new_signal
    }
}

impl<'a, T: 'a> SignalExt<'a> for Signal<'a, T> {
    fn react(&self) {
        self.0.react_fns.borrow().iter().for_each(|react_fn| {
            react_fn();
        });
    }
    fn guard(&self) -> SignalGuard<'a> {
        let mut result = vec![];
        self.collect_guards(&mut result);
        SignalGuard(result)
    }
    fn clone_box(&self) -> Box<dyn SignalExt<'a> + 'a> {
        Box::new(Signal(Rc::clone(&self.0)))
    }
    fn decrease_dirty(&self) {
        *self.0.dirty.borrow_mut() -= 1;
    }
    fn get_dirty(&self) -> isize {
        *self.0.dirty.borrow()
    }
    fn reset_explicitly_modified(&self) {
        *self.0.explicitly_modified.borrow_mut() = false;
    }
    fn collect_guards_recursive(&self, result: &mut Vec<SignalGuardInner<'a>>) {
        self.mark_dirty();
        result.push(SignalGuardInner(self.clone_box()));
        self.collect_and_iterate(&self.0.successors, |signal| {
            signal.collect_guards_recursive(result);
        });
    }
    fn collect_predecessors_recursive(&self, result: &mut Vec<SignalGuardInner<'a>>) {
        self.mark_dirty();
        result.push(SignalGuardInner(self.clone_box()));
        // Collect predecessors last so they drop last (react last)
        self.collect_and_iterate(&self.0.predecessors, |signal| {
            signal.collect_predecessors_recursive(result);
        });
    }
}

impl<T> Clone for Signal<'_, T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<'a, T> AsRef<Signal<'a, T>> for Signal<'a, T> {
    fn as_ref(&self) -> &Signal<'a, T> {
        self
    }
}

#[cfg(test)]
mod tests {
    use crate::api::LiftInto;

    use super::*;

    #[test]
    fn test_signal() {
        let a = Signal::new(0);
        let _a = a.map(|x| println!("a changed: {}", x));
        (a.send(100), a.send(5));
    }

    #[test]
    fn test_signal1() {
        let a = Signal::new(0);
        let b = a.map(|x| x * 2);
        let _b = b.map(|x| println!("b changed: {}", x));
        drop(b);
        a.send(100);
    }

    #[test]
    fn test_signal2() {
        let a = Signal::new(0);
        let b = a.map(|x| x * 2);
        let ab = a.with(&b);
        let _ab = ab.map(|(x, y)| println!("c changed: {} + {} = {}", x, y, x + y));
        (a.send(5), a.send(100));
    }

    #[test]
    fn test_signal3() {
        let a = Signal::new(0);
        let b = a.map(|x| x * 2);
        let ab = (&a, &b).lift();
        let _ab = ab.map(|(x, y)| println!("c changed: {} + {} = {}", x, y, x + y));
        (a.send(5), b.send(100));
    }

    #[test]
    fn test_signal4() {
        let a = Signal::new(0);
        let b = Signal::new(10);
        let c = Signal::new(20);
        let abc = [&a, &b, &c].lift();
        let _abc =
            abc.map(|[x, y, z]| println!("d changed: {} + {} + {} = {}", x, y, z, x + y + z));
        let d = Signal::new(0);
        let abcd = abc.with(&d);
        let _abcd = abcd.map(|numbers| {
            println!(
                "e changed: {} + {} + {} + {} = {}",
                numbers.0[0],
                numbers.0[1],
                numbers.0[2],
                numbers.1,
                numbers.0.iter().sum::<i32>() + numbers.1
            )
        });
        (a.send(5), b.send(15), c.send(25), abc.send([2, 3, 4]));
    }

    #[test]
    fn test_signal5() {
        let result = Signal::new(42);
        let source1 = result.comap(|x| x + 1);
        let source2 = source1.comap(|x| x * 2);

        let _observer_1 = result.map(|x| println!("result changed: {}", x));
        let _observer_2 = source1.map(|x| println!("source1 changed: {}", x));
        let _observer_3 = source2.map(|x| println!("source2 changed: {}", x));

        println!("--- Sending to source1 ---");
        source1.send(100);
        println!("--- Sending to source2 ---");
        source2.send(200);
        println!("--- Sending to source1 and source2 ---");
        (source1.send(300), source2.send(400));
    }

    #[test]
    fn test_promap_forward() {
        let source = Signal::new(10);
        let derived = source.promap(|x| x * 2, |y| y / 2);

        let _ = derived.map(|x| println!("derived changed: {}", x));
        source.send(5);
    }

    #[test]
    fn test_promap_backward() {
        let source = Signal::new(10);
        let derived = source.promap(|x| x * 2, |y| y / 2);

        let _ = source.map(|x| println!("source changed: {}", x));
        let _ = derived.map(|x| println!("derived changed: {}", x));
        derived.send(50);
    }

    #[test]
    fn test_promap_bidirectional() {
        let a = Signal::new(10);
        let b = a.promap(|x| x * 2, |y| y / 2);
        let c = b.promap(|x| x + 3, |y| y - 3);

        let _observer_a = a.map(|x| println!("a changed: {}", x));
        let _observer_b = b.map(|x| println!("b changed: {}", x));
        let _observer_c = c.map(|x| println!("c changed: {}", x));

        println!("--- Sending to a ---");
        a.send(5);
        println!("--- Sending to c ---");
        c.send(13);
        println!("--- Sending to b ---");
        b.send(10);
        println!("--- Sending to a and c ---");
        (a.send(20), c.send(50));
        println!("--- Sending to b and c ---");
        (b.send(30), c.send(60));
    }

    #[test]
    fn test_high_order_signal() {
        let source = Signal::new(0);
        let derived = source.map(|x| Signal::new(x + 1));
        let _ = derived.map(|s| s.send(233));
        let _ = derived.map(|s| s.map(|x| println!("derived changed: {}", x)));
        source.send(5);
    }
}
