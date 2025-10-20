use std::{cell::RefCell, iter, rc::Rc};

use crate::api::Liftable;

pub(crate) trait SignalExt<'a> {
    fn react(&self);
    fn guard(&self) -> SignalGuard<'a>;
    fn decrease_dirty(&self);
    fn get_dirty(&self) -> isize;
    fn clone_box(&self) -> Box<dyn SignalExt<'a> + 'a>;
    fn collect_guards_recursive(&self, result: &mut Vec<SignalGuardInner<'a>>);
    fn reset_explicitly_modified(&self);
}

pub(crate) struct SignalGuardInner<'a>(Box<dyn SignalExt<'a> + 'a>);

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

pub struct SignalInner<'a, T> {
    pub(crate) value: RefCell<T>,
    pub(crate) react_fns: RefCell<Vec<Box<dyn Fn() + 'a>>>,
    pub(crate) successors: RefCell<Vec<Box<dyn SignalExt<'a> + 'a>>>,
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
        let cloned_new_signal = Box::new(new_signal.clone());
        let result_new_signal = new_signal.clone();
        let source_for_closure = Rc::clone(&self.0);

        let react_fn = Box::new(move || {
            // Only update if the signal wasn't explicitly modified
            if !*new_signal.0.explicitly_modified.borrow() {
                let new_value = f(&source_for_closure.value.borrow());
                new_signal.modify(|v| *v = new_value);
            }
        });

        self.0.react_fns.borrow_mut().push(react_fn);
        self.0.successors.borrow_mut().push(cloned_new_signal);
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
        let cloned_new_signal = new_signal.clone();
        let cloned_new_signal_ = Box::new(new_signal.clone());
        let cloned_new_signal__ = Box::new(new_signal.clone());
        let result_new_signal = new_signal.clone();
        let source_for_closure_self = Rc::clone(&self.0);
        let source_for_closure_another = Rc::clone(&another.0);

        let react_fn_self = Box::new(move || {
            if !*new_signal.0.explicitly_modified.borrow() {
                new_signal.modify(|v| v.0 = source_for_closure_self.value.borrow().clone());
            }
        });
        let react_fn_another = Box::new(move || {
            if !*cloned_new_signal.0.explicitly_modified.borrow() {
                cloned_new_signal
                    .modify(|v| v.1 = source_for_closure_another.value.borrow().clone());
            }
        });
        self.0.react_fns.borrow_mut().push(react_fn_self);
        another.0.react_fns.borrow_mut().push(react_fn_another);
        self.0.successors.borrow_mut().push(cloned_new_signal_);
        another.0.successors.borrow_mut().push(cloned_new_signal__);
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
                let new_signal_clone = new_signal.clone();
                let cloned_new_signal_box = Box::new(new_signal.clone());
                let source_for_closure = Rc::clone(&signal.0);

                let react_fn = Box::new(move || {
                    if !*new_signal_clone.0.explicitly_modified.borrow() {
                        new_signal_clone.modify(|v| {
                            v[index] = source_for_closure.value.borrow().clone();
                        });
                    }
                });

                signal.0.react_fns.borrow_mut().push(react_fn);
                signal.0.successors.borrow_mut().push(cloned_new_signal_box);
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

    fn collect_guards(&self, result: &mut Vec<SignalGuardInner<'a>>) {
        self.mark_dirty();
        result.push(SignalGuardInner(self.clone_box()));
        for s in self.0.successors.borrow().iter() {
            s.collect_guards_recursive(result);
        }
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
            let new_signal_clone = new_signal.clone();
            let cloned_new_signal_box = Box::new(new_signal.clone());
            let source_for_closure = Rc::clone(&signal.0);

            let react_fn = Box::new(move || {
                if !*new_signal_clone.0.explicitly_modified.borrow() {
                    new_signal_clone.modify(|v| {
                        v[index] = source_for_closure.value.borrow().clone();
                    });
                }
            });

            signal.0.react_fns.borrow_mut().push(react_fn);
            signal.0.successors.borrow_mut().push(cloned_new_signal_box);
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
        for s in self.0.successors.borrow().iter() {
            s.collect_guards_recursive(result);
        }
    }
}

impl<T> Clone for Signal<'_, T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
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
}
