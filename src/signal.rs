use std::{cell::RefCell, iter, rc::Rc};

use crate::api::Liftable;

pub(crate) trait SignalExt<'a> {
    fn react(&self);
    fn guard(&self) -> SignalGuard<'a>;
    fn decrease_dirty(&self);
    fn get_dirty(&self) -> isize;
    fn clone_box(&self) -> Box<dyn SignalExt<'a> + 'a>;
    fn collect_guards_recursive(&self, result: &mut Vec<SignalGuardInner<'a>>);
    fn collect_predecessors_recursive(&self, result: &mut Vec<SignalGuardInner<'a>>);
    fn reset_explicitly_modified(&self);
}

pub(crate) trait RefStrategy<'a> {
    type Ref<T: 'a>: 'a;
    fn new_ref<T: 'a>(inner: &Rc<SignalInner<'a, T>>) -> Self::Ref<T>;
    fn upgrade<T: 'a>(ref_: &Self::Ref<T>) -> Option<Rc<SignalInner<'a, T>>>;
}

pub(crate) struct WeakRefStrategy;

impl<'a> RefStrategy<'a> for WeakRefStrategy {
    type Ref<T: 'a> = std::rc::Weak<SignalInner<'a, T>>;

    fn new_ref<T: 'a>(inner: &Rc<SignalInner<'a, T>>) -> Self::Ref<T> {
        Rc::downgrade(inner)
    }

    fn upgrade<T: 'a>(ref_: &Self::Ref<T>) -> Option<Rc<SignalInner<'a, T>>> {
        ref_.upgrade()
    }
}

pub(crate) struct StrongRefStrategy;

impl<'a> RefStrategy<'a> for StrongRefStrategy {
    type Ref<T: 'a> = Rc<SignalInner<'a, T>>;

    fn new_ref<T: 'a>(inner: &Rc<SignalInner<'a, T>>) -> Self::Ref<T> {
        inner.clone()
    }

    fn upgrade<T: 'a>(ref_: &Self::Ref<T>) -> Option<Rc<SignalInner<'a, T>>> {
        Some(ref_.clone())
    }
}

pub(crate) struct WeakSignalRef<'a> {
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

/// The inner part of a signal guard
pub struct SignalGuardInner<'a>(Box<dyn SignalExt<'a> + 'a>);

/// Signal guard that triggers reactions on drop
#[allow(dead_code)]
#[allow(unused_must_use)]
pub struct SignalGuard<'a>(Vec<SignalGuardInner<'a>>);

impl<'a> SignalGuard<'a> {
    /// Combine two signal guards into one
    pub fn and(mut self, mut other: SignalGuard<'a>) -> SignalGuard<'a> {
        self.0.append(&mut other.0);
        self
    }
}

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

/// The inner data of a signal
pub struct SignalInner<'a, T> {
    pub(crate) value: RefCell<T>,
    pub(crate) react_fns: RefCell<Vec<Box<dyn Fn() + 'a>>>,
    pub(crate) successors: RefCell<Vec<WeakSignalRef<'a>>>,
    pub(crate) predecessors: RefCell<Vec<WeakSignalRef<'a>>>,
    pub(crate) dirty: RefCell<isize>,
    pub(crate) explicitly_modified: RefCell<bool>,
}

/// Signal representing a reactive value
pub struct Signal<'a, T>(pub(crate) Rc<SignalInner<'a, T>>);

impl<'a, T: 'a> Signal<'a, T> {
    /// Create a new signal with the given initial value
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

    /// Helper: Temporarily take value without cloning using MaybeUninit
    #[inline]
    fn take_value<U>(cell: &RefCell<U>) -> U {
        let mut temp = unsafe { std::mem::MaybeUninit::<U>::uninit().assume_init() };
        std::mem::swap(&mut *cell.borrow_mut(), &mut temp);
        temp
    }

    /// Send a new value to the signal
    ///
    /// This will replace the current value of the signal with the new value.
    /// The signals that depend on this signal will be notified and updated accordingly.
    ///
    /// It returns a `SignalGuard` that ensures reactions are processed when dropped and
    /// prevents premature reactions during multiple sends. (Batch updates)
    /// # Example
    /// ```rust
    /// let signal = Signal::new(0);
    /// signal.send(42); // sets the signal's value to 42
    ///
    /// signal.with(|v| println!("Signal value: {}", v));
    /// (signal.send(66), signal.send(100));
    /// // sets the signal's value to 100 and prints "Signal value: 100" only once
    /// ```
    pub fn send(&self, new_value: T) -> SignalGuard<'a> {
        self.modify(|v| *v = new_value);
        *self.0.explicitly_modified.borrow_mut() = true;
        self.guard()
    }

    /// Send a modification to the signal
    ///
    /// This will apply the provided function to modify the current value of the signal.
    /// The signals that depend on this signal will be notified and updated accordingly.
    ///
    /// It returns a `SignalGuard` that ensures reactions are processed when dropped and
    /// prevents premature reactions during multiple sends. (Batch updates)
    pub fn send_with<F>(&self, f: F) -> SignalGuard<'a>
    where
        F: FnOnce(&mut T),
    {
        self.modify(f);
        self.guard()
    }

    pub fn set(&mut self, signal: Signal<'a, T>) {
        self.0 = signal.0;
    }

    /// Map the signal to a new signal
    ///
    /// This creates a new signal that depends on the current signal.
    /// Changes to the source signal will propagate to the new signal.
    ///
    /// # Example
    /// ```rust
    /// let a = Signal::new(10);
    /// let b = a.map(|x| x * 2);
    /// let _observer = b.map(|x| println!("b changed: {}", x));
    /// a.send(5); // prints "b changed: 10"
    /// ```
    pub fn map<U: 'a, F>(&self, f: F) -> Signal<'a, U>
    where
        F: Fn(&T) -> U + 'a,
    {
        self.map_ref::<U, F, WeakRefStrategy>(f)
    }

    /// Map the signal to a new signal with strong references
    ///
    /// This creates a new signal that depends on the current signal.
    /// Changes to the source signal will propagate to the new signal.
    /// This mapping uses strong references.
    ///
    /// # Example
    /// ```rust
    /// let a = Signal::new(10);
    /// let b = a.map(|x| x * 2);
    /// b.with(|x| println!("b changed: {}", x));
    /// a.send(5); // prints "b changed: 10"
    /// ```
    pub fn with<U: 'a, F>(&self, f: F) -> Signal<'a, U>
    where
        F: Fn(&T) -> U + 'a,
    {
        self.map_ref::<U, F, StrongRefStrategy>(f)
    }

    fn map_ref<U: 'a, F, S: RefStrategy<'a>>(&self, f: F) -> Signal<'a, U>
    where
        F: Fn(&T) -> U + 'a,
    {
        let new_signal = Signal::new(f(&self.0.value.borrow()));
        let result_new_signal = new_signal.clone();

        let new_signal_ref = S::new_ref(&new_signal.0);
        let source_ref = S::new_ref(&self.0);

        let react_fn = Box::new(move || {
            if let Some(new_sig_inner) = S::upgrade(&new_signal_ref) {
                if !*new_sig_inner.explicitly_modified.borrow() {
                    if let Some(src_inner) = S::upgrade(&source_ref) {
                        let new_value = f(&src_inner.value.borrow());
                        *new_sig_inner.value.borrow_mut() = new_value;
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

    /// Map the signal contravariantly to a new signal
    ///
    /// This creates a new signal that the current signal depends on.
    /// Changes to the new signal will propagate back to the original signal.
    /// It is inspired by the concept of contravariant functors in category theory.
    ///
    /// # Example
    /// ```rust
    /// let result = Signal::new(42);
    /// let source = result.contramap(|x| x * 2);
    /// result.with(|x| println!("result changed: {}", x));
    /// source.with(|x| println!("source changed: {}", x));
    /// source.send(100);
    /// // prints "source changed: 100" and "result changed: 50"
    /// ```
    pub fn contramap<F, U>(&self, f: F) -> Signal<'a, U>
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

        new_signal
            .0
            .predecessors
            .borrow_mut()
            .push(WeakSignalRef::new(self));

        result_new_signal
    }

    /// Map the signal bidirectionally to a new signal
    ///
    /// This creates a new signal that depends on the current signal,
    /// and the current signal also depends on the new signal.
    /// Changes to either signal will propagate to the other signal.
    /// It is inspired by the concept of profunctors in category theory.
    ///
    /// # Example
    /// ```rust
    /// let a = Signal::new(10);
    /// let b = a.promap(|x| x * 2, |y| y / 2);
    /// a.with(|x| println!("a changed: {}", x));
    /// b.with(|x| println!("b changed: {}", x));
    /// a.send(5); // prints "a changed: 5" and "b changed: 10"
    /// b.send(50); // prints "b changed: 50" and "a changed: 25"
    /// ```
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

    /// Combine two signals into one
    ///
    /// This combines two signals into a new signal that holds a tuple of their values.
    /// Changes to either signal will propagate to the new combined signal.
    ///
    /// # Example
    /// ```rust
    /// let a = Signal::new(10);
    /// let b = a.map(|x| x * 2);
    /// let ab = a.combine(&b);
    /// ab.with(|(x, y)| println!("c changed: {} + {} = {}", x, y, x + y));
    /// a.send(5); // prints "c changed: 5 + 10 = 15"
    /// ```
    pub fn combine<S>(&self, another: S) -> Signal<'a, (T, S::Inner)>
    where
        S: Liftable<'a>,
        S::Inner: 'a,
        T: 'a,
    {
        self.combine_ref::<S, WeakRefStrategy>(another)
    }

    /// Combine two signals into one with strong references
    ///
    /// This combines two signals into a new signal that holds a tuple of their values.
    /// Changes to either signal will propagate to the new combined signal.
    /// This combination uses strong references.
    ///
    /// # Example
    /// ```rust
    /// let a = Signal::new(10);
    /// let b = a.map(|x| x * 2);
    /// a.and(&b).with(|(x, y)| println!("c changed: {} + {} = {}", x, y, x + y));
    /// a.send(5); // prints "c changed: 5 + 10 = 15"
    /// ```
    pub fn and<S>(&self, another: S) -> Signal<'a, (T, S::Inner)>
    where
        S: Liftable<'a>,
        S::Inner: 'a,
        T: 'a,
    {
        self.combine_ref::<S, StrongRefStrategy>(another)
    }

    fn combine_ref<S: Liftable<'a>, Strat: RefStrategy<'a>>(
        &self,
        another: S,
    ) -> Signal<'a, (T, S::Inner)>
    where
        S::Inner: 'a,
        T: 'a,
    {
        let another = another.as_ref();

        // Take values temporarily, create signal, restore values
        let temp_val_0 = Self::take_value(&self.0.value);
        let temp_val_1 = Self::take_value(&another.0.value);
        let new_signal = Signal::new((temp_val_0, temp_val_1));
        std::mem::swap(
            &mut *self.0.value.borrow_mut(),
            &mut new_signal.0.value.borrow_mut().0,
        );
        std::mem::swap(
            &mut *another.0.value.borrow_mut(),
            &mut new_signal.0.value.borrow_mut().1,
        );

        let result_new_signal = new_signal.clone();

        // Register reaction for first source
        let new_signal_ref = Strat::new_ref(&new_signal.0);
        let self_ref = Strat::new_ref(&self.0);
        let react_fn_self = Box::new(move || {
            if let (Some(new_sig), Some(src)) =
                (Strat::upgrade(&new_signal_ref), Strat::upgrade(&self_ref))
            {
                if !*new_sig.explicitly_modified.borrow() {
                    std::mem::swap(
                        &mut *src.value.borrow_mut(),
                        &mut new_sig.value.borrow_mut().0,
                    );
                }
            }
        });
        self.0.react_fns.borrow_mut().push(react_fn_self);
        self.0
            .successors
            .borrow_mut()
            .push(WeakSignalRef::new(&result_new_signal));

        // Register reaction for second source
        let new_signal_ref_2 = Strat::new_ref(&new_signal.0);
        let another_ref = Strat::new_ref(&another.0);
        let react_fn_another = Box::new(move || {
            if let (Some(new_sig), Some(src)) = (
                Strat::upgrade(&new_signal_ref_2),
                Strat::upgrade(&another_ref),
            ) {
                if !*new_sig.explicitly_modified.borrow() {
                    std::mem::swap(
                        &mut *src.value.borrow_mut(),
                        &mut new_sig.value.borrow_mut().1,
                    );
                }
            }
        });
        another.0.react_fns.borrow_mut().push(react_fn_another);
        another
            .0
            .successors
            .borrow_mut()
            .push(WeakSignalRef::new(&result_new_signal));

        result_new_signal
    }

    /// Extend the signal with a vector of signals
    ///
    /// This creates a new signal that depends on the current signal and the provided signals.
    /// Changes to any of the source signals will propagate to the new signal.
    ///
    /// # Example
    /// ```rust
    /// let a = Signal::new(1);
    /// let b = Signal::new(2);
    /// let c = Signal::new(3);
    /// let d = a.extend(vec![b, c]);
    /// d.with(|values| println!("d changed: {:?}", values));
    /// a.send(10); // prints "d changed: [10, 2, 3]"
    /// (b.send(20), c.send(30)); // prints "d changed: [10, 20, 30]"
    /// ```
    pub fn extend<S>(&self, others: impl IntoIterator<Item = S>) -> Signal<'a, Vec<T>>
    where
        S: Liftable<'a, Inner = T>,
        T: 'a,
    {
        self.extend_ref::<S, WeakRefStrategy>(others)
    }

    /// Extend the signal with a vector of signals with strong references
    ///
    /// This creates a new signal that depends on the current signal and the provided signals.
    /// Changes to any of the source signals will propagate to the new signal.
    /// It uses strong references.
    ///
    /// # Example
    /// ```rust
    /// let a = Signal::new(1);
    /// let b = Signal::new(2);
    /// let c = Signal::new(3);
    /// a.follow(vec![b, c]).with(|values| println!("d changed: {:?}", values));
    /// a.send(10); // prints "d changed: [10, 2, 3]"
    /// (b.send(20), c.send(30)); // prints "d changed: [10, 20, 30]"
    /// ```
    pub fn follow<S>(&self, others: impl IntoIterator<Item = S>) -> Signal<'a, Vec<T>>
    where
        S: Liftable<'a, Inner = T>,
        T: 'a,
    {
        self.extend_ref::<S, StrongRefStrategy>(others)
    }

    fn extend_ref<S, Strat: RefStrategy<'a>>(
        &self,
        others: impl IntoIterator<Item = S>,
    ) -> Signal<'a, Vec<T>>
    where
        S: Liftable<'a, Inner = T>,
        T: 'a,
    {
        let others_signals: Vec<Signal<'a, T>> =
            others.into_iter().map(|s| s.as_ref().clone()).collect();

        // Collect values using take_value helper - no cloning!
        let all_signals: Vec<&Signal<'a, T>> =
            iter::once(self).chain(others_signals.iter()).collect();

        let temp_values: Vec<T> = all_signals
            .iter()
            .map(|s| Self::take_value(&s.0.value))
            .collect();
        let new_signal: Signal<'a, Vec<T>> = Signal::new(temp_values);

        // Restore original values by swapping back
        for (index, signal) in all_signals.iter().enumerate() {
            std::mem::swap(
                &mut *signal.0.value.borrow_mut(),
                &mut new_signal.0.value.borrow_mut()[index],
            );
        }

        let result_new_signal = new_signal.clone();

        iter::once(self)
            .chain(others_signals.iter())
            .enumerate()
            .for_each(|(index, signal)| {
                let new_signal_ref = Strat::new_ref(&new_signal.0);
                let source_ref = Strat::new_ref(&signal.0);

                let react_fn = Box::new(move || {
                    if let Some(new_sig) = Strat::upgrade(&new_signal_ref) {
                        if !*new_sig.explicitly_modified.borrow() {
                            if let Some(src) = Strat::upgrade(&source_ref) {
                                // Swap values instead of cloning (during reaction only)
                                std::mem::swap(
                                    &mut new_sig.value.borrow_mut()[index],
                                    &mut *src.value.borrow_mut(),
                                );
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

    /// Let this signal depend on another signal
    ///
    /// This synchronizes the value of this signal with the value of the dependency signal.
    /// Whenever the dependency signal changes, this signal will be updated to match its value.
    ///
    /// The returned signal is exactly the argument `dependency`. You can break the dependency chain
    /// by dropping the returned signal if `dependency` is weakly referenced.
    ///
    /// # Example
    /// ```rust
    /// let a = Signal::new(1);
    /// let mut b = Signal::new(2);
    /// b = a.depend(b);
    /// a.with(|v| println!("a changed: {}", v));
    /// b.send(3); // prints "a changed: 3"
    /// ```
    ///
    /// The example above is analogous to:
    /// ```rust
    /// let a = Signal::new(1);
    /// let b = a.map(|v| *v);
    /// b.with(|v| println!("b changed: {}", v));
    /// a.send(3); // prints "b changed: 3"
    /// ```
    pub fn depend(&self, dependency: Signal<'a, T>) -> Signal<'a, T>
    where
        T: Clone,
    {
        let self_weak = Rc::downgrade(&self.0);
        let dependency_weak = Rc::downgrade(&dependency.0);

        let react_fn = Box::new(move || {
            if let Some(dep) = dependency_weak.upgrade() {
                if let Some(target) = self_weak.upgrade() {
                    if !*target.explicitly_modified.borrow() {
                        // Swap values instead of cloning (during reaction only)
                        std::mem::swap(
                            &mut *target.value.borrow_mut(),
                            &mut *dep.value.borrow_mut(),
                        );
                    }
                }
            }
        });

        dependency.0.react_fns.borrow_mut().push(react_fn);
        dependency
            .0
            .successors
            .borrow_mut()
            .push(WeakSignalRef::new(self));
        dependency
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

    /// Lift an array of liftable items into a signal of an array
    ///
    /// This creates a new signal that depends on the provided liftable items.
    /// Changes to any of the source signals will propagate to the new signal.
    ///
    /// # Example
    /// ```rust
    /// let a = Signal::new(1);
    /// let b = Signal::new(2);
    /// let c = Signal::new(3);
    /// let abc = Signal::lift_from_array([a, b, c]);
    /// abc.with(|values| println!("abc changed: {:?}", values));
    /// (a.send(10), b.send(20), c.send(30)); // prints "abc changed: [10, 20, 30]"
    /// ```
    pub fn lift_from_array<S, const N: usize>(items: [S; N]) -> Signal<'a, [S::Inner; N]>
    where
        S: Liftable<'a>,
        S::Inner: 'a,
    {
        let signals: [Signal<'a, S::Inner>; N] = std::array::from_fn(|i| items[i].as_ref().clone());

        // Take values using helper - no cloning!
        let initial: [S::Inner; N] = std::array::from_fn(|i| Self::take_value(&signals[i].0.value));
        let new_signal: Signal<'a, [S::Inner; N]> = Signal::new(initial);

        // Restore original values by swapping back
        for (index, signal) in signals.iter().enumerate() {
            std::mem::swap(
                &mut *signal.0.value.borrow_mut(),
                &mut new_signal.0.value.borrow_mut()[index],
            );
        }

        let result_new_signal = new_signal.clone();

        for (index, signal) in signals.iter().enumerate() {
            let new_signal_weak = Rc::downgrade(&new_signal.0);
            let source_for_closure = Rc::downgrade(&signal.0);

            let react_fn = Box::new(move || {
                if let Some(new_sig) = new_signal_weak.upgrade() {
                    if !*new_sig.explicitly_modified.borrow() {
                        if let Some(source) = source_for_closure.upgrade() {
                            // Swap instead of cloning (during reaction only)
                            std::mem::swap(
                                &mut new_sig.value.borrow_mut()[index],
                                &mut *source.value.borrow_mut(),
                            );
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
        let ab = a.combine(&b);
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
        let abcd = abc.combine(&d);
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
        let source1 = result.contramap(|x| x + 1);
        let source2 = source1.contramap(|x| x * 2);

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
        a.send(20).and(c.send(50));
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

    #[test]
    fn test_depend() {
        let a = Signal::new(10);
        let b = Signal::new(10);
        let c = Signal::new(10);

        let _observer_a = a.map(|x| println!("a changed: {}", x));
        let _observer_b = b.map(|x| println!("b changed: {}", x));
        let _observer_c = c.map(|x| println!("c changed: {}", x));

        c.depend(b.with(|x| x * 2));
        b.depend(a.clone());

        a.send(42);
    }

    #[test]
    fn test_depend2() {
        let a = Signal::new(10);
        let b = a.map(|x| *x);
        let c = b.map(|x| *x);

        let _observer_a = a.map(|x| println!("a changed: {}", x));
        let _observer_b = b.map(|x| println!("b changed: {}", x));
        let _observer_c = c.map(|x| println!("c changed: {}", x));

        (a.send(42), b.send(88));
    }
}
