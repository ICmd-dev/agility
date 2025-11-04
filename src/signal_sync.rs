use std::{
    iter,
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicBool, AtomicIsize, Ordering},
    },
};

use crate::api::LiftableSync;

pub(crate) trait SignalExtSync<'a>: Send + Sync {
    fn react(&self);
    fn guard(&self) -> SignalGuardSync<'a>;
    fn decrease_dirty(&self);
    fn get_dirty(&self) -> isize;
    fn clone_box(&self) -> Box<dyn SignalExtSync<'a> + 'a>;
    fn collect_guards_recursive(&self, result: &mut Vec<SignalGuardInnerSync<'a>>);
    fn collect_predecessors_recursive(&self, result: &mut Vec<SignalGuardInnerSync<'a>>);
    fn reset_explicitly_modified(&self);
}

// Strategy trait for reference handling (thread-safe version)
pub(crate) trait RefStrategySync<'a>: Sized {
    type Ref<T: 'a + Send + Sync>: Send + Sync;

    fn new_ref<T: 'a + Send + Sync>(signal: &SignalSync<'a, T>) -> Self::Ref<T>;
    fn upgrade_ref<T: 'a + Send + Sync>(ref_: &Self::Ref<T>)
    -> Option<Arc<SignalInnerSync<'a, T>>>;
}

// Weak reference strategy - allows garbage collection
pub(crate) struct WeakRefStrategySync;

impl<'a> RefStrategySync<'a> for WeakRefStrategySync {
    type Ref<T: 'a + Send + Sync> = std::sync::Weak<SignalInnerSync<'a, T>>;

    fn new_ref<T: 'a + Send + Sync>(signal: &SignalSync<'a, T>) -> Self::Ref<T> {
        Arc::downgrade(&signal.0)
    }

    fn upgrade_ref<T: 'a + Send + Sync>(
        ref_: &Self::Ref<T>,
    ) -> Option<Arc<SignalInnerSync<'a, T>>> {
        ref_.upgrade()
    }
}

// Strong reference strategy - holds strong references
pub(crate) struct StrongRefStrategySync;

impl<'a> RefStrategySync<'a> for StrongRefStrategySync {
    type Ref<T: 'a + Send + Sync> = Arc<SignalInnerSync<'a, T>>;

    fn new_ref<T: 'a + Send + Sync>(signal: &SignalSync<'a, T>) -> Self::Ref<T> {
        signal.0.clone()
    }

    fn upgrade_ref<T: 'a + Send + Sync>(
        ref_: &Self::Ref<T>,
    ) -> Option<Arc<SignalInnerSync<'a, T>>> {
        Some(ref_.clone())
    }
}

// Helper struct to hold weak references that can be upgraded (thread-safe version)
pub(crate) struct WeakSignalRefSync<'a> {
    upgrade: Box<dyn Fn() -> Option<Box<dyn SignalExtSync<'a> + 'a>> + Send + Sync + 'a>,
}

impl<'a> WeakSignalRefSync<'a> {
    pub fn new<T: Send + Sync + 'a>(signal: &SignalSync<'a, T>) -> Self {
        let weak = Arc::downgrade(&signal.0);
        WeakSignalRefSync {
            upgrade: Box::new(move || {
                weak.upgrade()
                    .map(|arc| Box::new(SignalSync(arc)) as Box<dyn SignalExtSync<'a> + 'a>)
            }),
        }
    }

    pub fn upgrade(&self) -> Option<Box<dyn SignalExtSync<'a> + 'a>> {
        (self.upgrade)()
    }

    pub fn is_alive(&self) -> bool {
        self.upgrade().is_some()
    }
}

/// The inner part of the signal (thread-safe version)
pub struct SignalGuardInnerSync<'a>(Box<dyn SignalExtSync<'a> + 'a>);

/// Guard that ensures reactions are processed when dropped (thread-safe version)
#[allow(dead_code)]
#[allow(unused_must_use)]
pub struct SignalGuardSync<'a>(Vec<SignalGuardInnerSync<'a>>);

impl<'a> SignalGuardSync<'a> {
    /// Combine two signal guards into one
    pub fn and(mut self, mut other: SignalGuardSync<'a>) -> SignalGuardSync<'a> {
        self.0.append(&mut other.0);
        self
    }
}

impl<'a> Drop for SignalGuardInnerSync<'a> {
    fn drop(&mut self) {
        self.0.decrease_dirty();
        if self.0.get_dirty() == 0 {
            self.0.react();
            self.0.reset_explicitly_modified();
        }
    }
}

impl<'a> Drop for SignalGuardSync<'a> {
    fn drop(&mut self) {
        // First drop all inner guards (triggers immediate reactions)
        drop(std::mem::take(&mut self.0));
    }
}

/// The inner part of the signal (thread-safe version)
pub struct SignalInnerSync<'a, T> {
    pub(crate) value: Mutex<T>,
    pub(crate) react_fns: RwLock<Vec<Box<dyn Fn() + Send + Sync + 'a>>>,
    pub(crate) successors: RwLock<Vec<WeakSignalRefSync<'a>>>,
    pub(crate) predecessors: RwLock<Vec<WeakSignalRefSync<'a>>>,
    pub(crate) dirty: AtomicIsize,
    pub(crate) explicitly_modified: AtomicBool,
}

/// A signal type that is thread-safe
pub struct SignalSync<'a, T>(pub(crate) Arc<SignalInnerSync<'a, T>>);

impl<'a, T: Send + Sync + 'a> SignalSync<'a, T> {
    /// Helper: temporarily take a value from a Mutex using MaybeUninit swap
    fn take_value<U>(mutex: &Mutex<U>) -> U {
        let mut temp = unsafe { std::mem::MaybeUninit::<U>::uninit().assume_init() };
        std::mem::swap(&mut *mutex.lock().unwrap(), &mut temp);
        temp
    }

    /// Create a new signal with the given initial value
    pub fn new(initial: T) -> Self {
        let inner = Arc::new(SignalInnerSync {
            value: Mutex::new(initial),
            react_fns: RwLock::new(Vec::new()),
            successors: RwLock::new(Vec::new()),
            predecessors: RwLock::new(Vec::new()),
            dirty: AtomicIsize::new(0),
            explicitly_modified: AtomicBool::new(false),
        });
        SignalSync(inner)
    }

    /// Send a new value to the signal
    ///
    /// This will replace the current value of the signal with the new value.
    /// The signals that depend on this signal will be notified and updated accordingly.
    ///
    /// It returns a `SignalGuardSync` that ensures reactions are processed when dropped and
    /// prevents premature reactions during multiple sends. (Batch updates)
    /// # Example
    /// ```rust
    /// let signal = SignalSync::new(0);
    /// signal.send(42); // sets the signal's value to 42
    ///
    /// signal.with(|v| println!("Signal value: {}", v));
    /// (signal.send(66), signal.send(100));
    /// // sets the signal's value to 100 and prints "Signal value: 100" only once
    /// ```
    pub fn send(&self, new_value: T) -> SignalGuardSync<'a> {
        self.modify(|v| *v = new_value);
        self.0.explicitly_modified.store(true, Ordering::Release);
        self.guard()
    }

    /// Send a modification to the signal
    ///
    /// This will apply the provided function to modify the current value of the signal.
    /// The signals that depend on this signal will be notified and updated accordingly.
    ///
    /// It returns a `SignalGuardSync` that ensures reactions are processed when dropped and
    /// prevents premature reactions during multiple sends. (Batch updates)
    pub fn send_with<F>(&self, f: F) -> SignalGuardSync<'a>
    where
        F: FnOnce(&mut T),
    {
        self.modify(f);
        self.guard()
    }

    /// Map the signal to a new signal
    ///
    /// This creates a new signal that depends on the current signal.
    /// Changes to the source signal will propagate to the new signal.
    ///
    /// # Example
    /// ```rust
    /// let a = SignalSync::new(10);
    /// let b = a.map(|x| x * 2);
    /// let _observer = b.map(|x| println!("b changed: {}", x));
    /// a.send(5); // prints "b changed: 10"
    /// ```
    pub fn map<U: Send + Sync + 'a, F>(&self, f: F) -> SignalSync<'a, U>
    where
        F: Fn(&T) -> U + Send + Sync + 'a,
    {
        self.map_ref::<U, F, WeakRefStrategySync>(f)
    }

    /// Map the signal to a new signal with strong references
    ///
    /// This creates a new signal that depends on the current signal.
    /// Changes to the source signal will propagate to the new signal.
    /// This mapping uses strong references.
    ///
    /// # Example
    /// ```rust
    /// let a = SignalSync::new(10);
    /// let b = a.map(|x| x * 2);
    /// b.with(|x| println!("b changed: {}", x));
    /// a.send(5); // prints "b changed: 10"
    /// ```
    pub fn with<U: Send + Sync + 'a, F>(&self, f: F) -> SignalSync<'a, U>
    where
        F: Fn(&T) -> U + Send + Sync + 'a,
    {
        self.map_ref::<U, F, StrongRefStrategySync>(f)
    }

    fn map_ref<U: Send + Sync + 'a, F, S: RefStrategySync<'a>>(&self, f: F) -> SignalSync<'a, U>
    where
        F: Fn(&T) -> U + Send + Sync + 'a,
        S: 'a,
    {
        let new_signal = SignalSync::new(f(&self.0.value.lock().unwrap()));
        let result_new_signal = new_signal.clone();

        let new_signal_ref = S::new_ref(&new_signal);
        let source_ref = S::new_ref(self);

        let react_fn = Box::new(move || {
            if let Some(new_sig) = S::upgrade_ref(&new_signal_ref) {
                if !new_sig.explicitly_modified.load(Ordering::Acquire) {
                    if let Some(src) = S::upgrade_ref(&source_ref) {
                        let new_value = f(&src.value.lock().unwrap());
                        *new_sig.value.lock().unwrap() = new_value;
                    }
                }
            }
        });

        self.0.react_fns.write().unwrap().push(react_fn);
        self.0
            .successors
            .write()
            .unwrap()
            .push(WeakSignalRefSync::new(&new_signal));

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
    /// let result = SignalSync::new(42);
    /// let source = result.contramap(|x| x * 2);
    /// result.with(|x| println!("result changed: {}", x));
    /// source.with(|x| println!("source changed: {}", x));
    /// source.send(100);
    /// // prints "source changed: 100" and "result changed: 50"
    /// ```
    pub fn contramap<F, U>(&self, f: F) -> SignalSync<'a, U>
    where
        F: Fn(&U) -> T + Send + Sync + 'a,
        U: Default + Send + Sync + 'a,
    {
        let new_signal = SignalSync::new(U::default());
        let result_new_signal = new_signal.clone();
        let source_inner = Arc::downgrade(&self.0);
        let new_signal_rc = Arc::downgrade(&new_signal.0);

        let react_fn = Box::new(move || {
            if let Some(new_sig) = new_signal_rc.upgrade() {
                if new_sig.explicitly_modified.load(Ordering::Acquire) {
                    let u_value_ref = new_sig.value.lock().unwrap();
                    let t_value = f(&u_value_ref);
                    drop(u_value_ref);

                    if let Some(source) = source_inner.upgrade() {
                        *source.value.lock().unwrap() = t_value;
                        source.explicitly_modified.store(true, Ordering::Release);
                    }
                }
            }
        });
        new_signal.0.react_fns.write().unwrap().push(react_fn);

        // Register self (source/result) as a predecessor of new_signal (backward dependency)
        new_signal
            .0
            .predecessors
            .write()
            .unwrap()
            .push(WeakSignalRefSync::new(self));

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
    /// let a = SignalSync::new(10);
    /// let b = a.promap(|x| x * 2, |y| y / 2);
    /// a.with(|x| println!("a changed: {}", x));
    /// b.with(|x| println!("b changed: {}", x));
    /// a.send(5); // prints "a changed: 5" and "b changed: 10"
    /// b.send(50); // prints "b changed: 50" and "a changed: 25"
    /// ```
    pub fn promap<F, G, U>(&self, f: F, g: G) -> SignalSync<'a, U>
    where
        F: Fn(&T) -> U + Send + Sync + 'a,
        G: Fn(&U) -> T + Send + Sync + 'a,
        U: Default + Send + Sync + 'a,
    {
        let new_signal = SignalSync::new(U::default());
        let result_new_signal = new_signal.clone();
        let source_weak = Arc::downgrade(&self.0);
        let new_signal_weak = Arc::downgrade(&new_signal.0);

        // Forward reaction: T -> U (covariant)
        let source_inner = source_weak.clone();
        let new_signal_rc = new_signal_weak.clone();
        let forward_react_fn = Box::new(move || {
            if let Some(new_sig) = new_signal_rc.upgrade() {
                if !new_sig.explicitly_modified.load(Ordering::Acquire) {
                    if let Some(source) = source_inner.upgrade() {
                        let t_value = source.value.lock().unwrap();
                        let u_value = f(&t_value);
                        drop(t_value);
                        *new_sig.value.lock().unwrap() = u_value;
                    }
                }
            }
        });

        self.0.react_fns.write().unwrap().push(forward_react_fn);
        self.0
            .successors
            .write()
            .unwrap()
            .push(WeakSignalRefSync::new(&new_signal));

        // Backward reaction: U -> T (contravariant)
        let new_signal_rc_back = new_signal_weak.clone();
        let source_inner_back = source_weak.clone();

        let backward_react_fn = Box::new(move || {
            if let Some(new_sig) = new_signal_rc_back.upgrade() {
                if new_sig.explicitly_modified.load(Ordering::Acquire) {
                    let u_value_ref = new_sig.value.lock().unwrap();
                    let t_value = g(&u_value_ref);
                    drop(u_value_ref);

                    if let Some(source) = source_inner_back.upgrade() {
                        *source.value.lock().unwrap() = t_value;
                        source.explicitly_modified.store(true, Ordering::Release);
                    }
                }
            }
        });
        new_signal
            .0
            .react_fns
            .write()
            .unwrap()
            .push(backward_react_fn);

        new_signal
            .0
            .predecessors
            .write()
            .unwrap()
            .push(WeakSignalRefSync::new(self));

        result_new_signal
    }

    /// Combine two signals into one
    ///
    /// This combines two signals into a new signal that holds a tuple of their values.
    /// Changes to either signal will propagate to the new combined signal.
    ///
    /// # Example
    /// ```rust
    /// let a = SignalSync::new(10);
    /// let b = a.map(|x| x * 2);
    /// let ab = a.combine(&b);
    /// ab.with(|(x, y)| println!("c changed: {} + {} = {}", x, y, x + y));
    /// a.send(5); // prints "c changed: 5 + 10 = 15"
    /// ```
    pub fn combine<S>(&self, another: S) -> SignalSync<'a, (T, S::Inner)>
    where
        S: LiftableSync<'a>,
        S::Inner: Send + Sync + 'a,
        T: Send + Sync,
    {
        self.combine_ref::<S, WeakRefStrategySync>(another)
    }

    /// Combine two signals into one with strong references
    ///
    /// This combines two signals into a new signal that holds a tuple of their values.
    /// Changes to either signal will propagate to the new combined signal.
    /// This combination uses strong references.
    ///
    /// # Example
    /// ```rust
    /// let a = SignalSync::new(10);
    /// let b = a.map(|x| x * 2);
    /// a.and(&b).with(|(x, y)| println!("c changed: {} + {} = {}", x, y, x + y));
    /// a.send(5); // prints "c changed: 5 + 10 = 15"
    /// ```
    pub fn and<S>(&self, another: S) -> SignalSync<'a, (T, S::Inner)>
    where
        S: LiftableSync<'a>,
        S::Inner: Send + Sync + 'a,
        T: Send + Sync,
    {
        self.combine_ref::<S, StrongRefStrategySync>(another)
    }

    fn combine_ref<S, St: RefStrategySync<'a>>(&self, another: S) -> SignalSync<'a, (T, S::Inner)>
    where
        S: LiftableSync<'a>,
        S::Inner: Send + Sync + 'a,
        T: Send + Sync,
        St: 'a,
    {
        let another = another.as_ref();

        // Take values using helper - no cloning!
        let temp_val_0 = Self::take_value(&self.0.value);
        let temp_val_1 = Self::take_value(&another.0.value);
        let new_signal = SignalSync::new((temp_val_0, temp_val_1));

        // Restore original values by swapping back from the new signal
        std::mem::swap(
            &mut *self.0.value.lock().unwrap(),
            &mut new_signal.0.value.lock().unwrap().0,
        );
        std::mem::swap(
            &mut *another.0.value.lock().unwrap(),
            &mut new_signal.0.value.lock().unwrap().1,
        );

        let result_new_signal = new_signal.clone();

        let new_signal_ref = St::new_ref(&new_signal);
        let source_self_ref = St::new_ref(self);

        let react_fn_self = Box::new(move || {
            if let Some(new_sig) = St::upgrade_ref(&new_signal_ref) {
                if !new_sig.explicitly_modified.load(Ordering::Acquire) {
                    if let Some(source) = St::upgrade_ref(&source_self_ref) {
                        // Swap values instead of cloning (during reaction only)
                        std::mem::swap(
                            &mut *source.value.lock().unwrap(),
                            &mut new_sig.value.lock().unwrap().0,
                        );
                    }
                }
            }
        });

        let new_signal_ref_2 = St::new_ref(&new_signal);
        let source_another_ref_2 = St::new_ref(another);
        let react_fn_another = Box::new(move || {
            if let Some(new_sig) = St::upgrade_ref(&new_signal_ref_2) {
                if !new_sig.explicitly_modified.load(Ordering::Acquire) {
                    if let Some(source) = St::upgrade_ref(&source_another_ref_2) {
                        // Swap values instead of cloning (during reaction only)
                        std::mem::swap(
                            &mut *source.value.lock().unwrap(),
                            &mut new_sig.value.lock().unwrap().1,
                        );
                    }
                }
            }
        });

        self.0.react_fns.write().unwrap().push(react_fn_self);
        another.0.react_fns.write().unwrap().push(react_fn_another);
        self.0
            .successors
            .write()
            .unwrap()
            .push(WeakSignalRefSync::new(&result_new_signal));
        another
            .0
            .successors
            .write()
            .unwrap()
            .push(WeakSignalRefSync::new(&result_new_signal));
        result_new_signal
    }

    /// Extend the signal with a vector of signals
    ///
    /// This creates a new signal that depends on the current signal and the provided signals.
    /// Changes to any of the source signals will propagate to the new signal.
    ///
    /// # Example
    /// ```rust
    /// let a = SignalSync::new(1);
    /// let b = SignalSync::new(2);
    /// let c = SignalSync::new(3);
    /// let d = a.extend(vec![b, c]);
    /// d.with(|values| println!("d changed: {:?}", values));
    /// a.send(10); // prints "d changed: [10, 2, 3]"
    /// (b.send(20), c.send(30)); // prints "d changed: [10, 20, 30]"
    /// ```
    pub fn extend<S>(&self, others: impl IntoIterator<Item = S>) -> SignalSync<'a, Vec<T>>
    where
        S: LiftableSync<'a, Inner = T>,
        T: Send + Sync,
    {
        self.extend_ref::<S, WeakRefStrategySync>(others)
    }

    /// Extend the signal with a vector of signals with strong references
    ///
    /// This creates a new signal that depends on the current signal and the provided signals.
    /// Changes to any of the source signals will propagate to the new signal.
    /// It uses strong references.
    ///
    /// # Example
    /// ```rust
    /// let a = SignalSync::new(1);
    /// let b = SignalSync::new(2);
    /// let c = SignalSync::new(3);
    /// a.follow(vec![b, c]).with(|values| println!("d changed: {:?}", values));
    /// a.send(10); // prints "d changed: [10, 2, 3]"
    /// (b.send(20), c.send(30)); // prints "d changed: [10, 20, 30]"
    /// ```
    pub fn follow<S>(&self, others: impl IntoIterator<Item = S>) -> SignalSync<'a, Vec<T>>
    where
        S: LiftableSync<'a, Inner = T>,
        T: Send + Sync,
    {
        self.extend_ref::<S, StrongRefStrategySync>(others)
    }

    fn extend_ref<S, St: RefStrategySync<'a>>(
        &self,
        others: impl IntoIterator<Item = S>,
    ) -> SignalSync<'a, Vec<T>>
    where
        S: LiftableSync<'a, Inner = T>,
        T: Send + Sync,
        St: 'a,
    {
        let others_signals: Vec<SignalSync<'a, T>> =
            others.into_iter().map(|s| s.as_ref().clone()).collect();

        // Collect values using helper - no cloning!
        let all_signals: Vec<&SignalSync<'a, T>> =
            iter::once(self).chain(others_signals.iter()).collect();
        let temp_values: Vec<T> = all_signals
            .iter()
            .map(|s| Self::take_value(&s.0.value))
            .collect();
        let new_signal: SignalSync<'a, Vec<T>> = SignalSync::new(temp_values);

        // Restore original values by swapping back
        for (index, signal) in all_signals.iter().enumerate() {
            std::mem::swap(
                &mut *signal.0.value.lock().unwrap(),
                &mut new_signal.0.value.lock().unwrap()[index],
            );
        }

        let result_new_signal = new_signal.clone();

        iter::once(self)
            .chain(others_signals.iter())
            .enumerate()
            .for_each(|(index, signal)| {
                let new_signal_ref = St::new_ref(&new_signal);
                let source_ref = St::new_ref(signal);

                let react_fn = Box::new(move || {
                    if let Some(new_sig) = St::upgrade_ref(&new_signal_ref) {
                        if !new_sig.explicitly_modified.load(Ordering::Acquire) {
                            if let Some(source) = St::upgrade_ref(&source_ref) {
                                // Swap values instead of cloning (during reaction only)
                                std::mem::swap(
                                    &mut new_sig.value.lock().unwrap()[index],
                                    &mut *source.value.lock().unwrap(),
                                );
                            }
                        }
                    }
                });

                signal.0.react_fns.write().unwrap().push(react_fn);
                signal
                    .0
                    .successors
                    .write()
                    .unwrap()
                    .push(WeakSignalRefSync::new(&new_signal));
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
    /// let a = SignalSync::new(1);
    /// let mut b = SignalSync::new(2);
    /// b = a.depend(b);
    /// a.with(|v| println!("a changed: {}", v));
    /// b.send(3); // prints "a changed: 3"
    /// ```
    ///
    /// The example above is analogous to:
    /// ```rust
    /// let a = SignalSync::new(1);
    /// let b = a.map(|v| *v);
    /// b.with(|v| println!("b changed: {}", v));
    /// a.send(3); // prints "b changed: 3"
    /// ```
    pub fn depend(&self, dependency: SignalSync<'a, T>) -> SignalSync<'a, T>
    where
        T: Default + Send + Sync,
    {
        let self_weak = Arc::downgrade(&self.0);
        let dependency_weak = Arc::downgrade(&dependency.0);

        let react_fn = Box::new(move || {
            if let Some(dep) = dependency_weak.upgrade() {
                if let Some(target) = self_weak.upgrade() {
                    if !target.explicitly_modified.load(Ordering::Acquire) {
                        // Swap values instead of cloning
                        std::mem::swap(
                            &mut *target.value.lock().unwrap(),
                            &mut *dep.value.lock().unwrap(),
                        );
                    }
                }
            }
        });

        dependency.0.react_fns.write().unwrap().push(react_fn);
        dependency
            .0
            .successors
            .write()
            .unwrap()
            .push(WeakSignalRefSync::new(self));
        dependency
    }

    /// Apply a modification function to the stored value (thread-safe)
    pub(crate) fn modify(&self, f: impl FnOnce(&mut T)) {
        let mut value = self.0.value.lock().unwrap();
        f(&mut value);
    }

    fn mark_dirty(&self) {
        self.0.dirty.fetch_add(1, Ordering::SeqCst);
    }

    fn collect_and_iterate<F>(&self, refs: &RwLock<Vec<WeakSignalRefSync<'a>>>, mut callback: F)
    where
        F: FnMut(&dyn SignalExtSync<'a>),
    {
        let signals_to_process: Vec<_> = {
            let mut refs_write = refs.write().unwrap();
            refs_write.retain(|s| s.is_alive());
            refs_write.iter().filter_map(|s| s.upgrade()).collect()
        };

        for signal in signals_to_process {
            callback(&*signal);
        }
    }

    fn collect_guards(&self, result: &mut Vec<SignalGuardInnerSync<'a>>) {
        self.mark_dirty();
        result.push(SignalGuardInnerSync(self.clone_box()));
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
    /// let a = SignalSync::new(1);
    /// let b = SignalSync::new(2);
    /// let c = SignalSync::new(3);
    /// let abc = SignalSync::lift_from_array([a, b, c]);
    /// abc.with(|values| println!("abc changed: {:?}", values));
    /// (a.send(10), b.send(20), c.send(30)); // prints "abc changed: [10, 20, 30]"
    /// ```
    pub fn lift_from_array<S, const N: usize>(items: [S; N]) -> SignalSync<'a, [S::Inner; N]>
    where
        S: LiftableSync<'a>,
        S::Inner: Send + Sync + 'a,
    {
        let signals: [SignalSync<'a, S::Inner>; N] =
            std::array::from_fn(|i| items[i].as_ref().clone());

        // Take values using helper - no cloning!
        let initial: [S::Inner; N] = std::array::from_fn(|i| Self::take_value(&signals[i].0.value));
        let new_signal: SignalSync<'a, [S::Inner; N]> = SignalSync::new(initial);

        // Restore original values by swapping back
        for (index, signal) in signals.iter().enumerate() {
            std::mem::swap(
                &mut *signal.0.value.lock().unwrap(),
                &mut new_signal.0.value.lock().unwrap()[index],
            );
        }

        let result_new_signal = new_signal.clone();

        for (index, signal) in signals.iter().enumerate() {
            let new_signal_weak = Arc::downgrade(&new_signal.0);
            let source_for_closure = Arc::downgrade(&signal.0);

            let react_fn = Box::new(move || {
                if let Some(new_sig) = new_signal_weak.upgrade() {
                    if !new_sig.explicitly_modified.load(Ordering::Acquire) {
                        if let Some(source) = source_for_closure.upgrade() {
                            // Swap instead of cloning (during reaction only)
                            std::mem::swap(
                                &mut new_sig.value.lock().unwrap()[index],
                                &mut *source.value.lock().unwrap(),
                            );
                        }
                    }
                }
            });

            signal.0.react_fns.write().unwrap().push(react_fn);
            signal
                .0
                .successors
                .write()
                .unwrap()
                .push(WeakSignalRefSync::new(&new_signal));
        }

        result_new_signal
    }
}

impl<'a, T: Send + Sync + 'a> SignalExtSync<'a> for SignalSync<'a, T> {
    fn react(&self) {
        self.0
            .react_fns
            .read()
            .unwrap()
            .iter()
            .for_each(|react_fn| {
                react_fn();
            });
    }
    fn guard(&self) -> SignalGuardSync<'a> {
        let mut result = vec![];
        self.collect_guards(&mut result);
        SignalGuardSync(result)
    }
    fn clone_box(&self) -> Box<dyn SignalExtSync<'a> + 'a> {
        Box::new(SignalSync(Arc::clone(&self.0)))
    }
    fn decrease_dirty(&self) {
        self.0.dirty.fetch_sub(1, Ordering::SeqCst);
    }
    fn get_dirty(&self) -> isize {
        self.0.dirty.load(Ordering::SeqCst)
    }
    fn reset_explicitly_modified(&self) {
        self.0.explicitly_modified.store(false, Ordering::Release);
    }
    fn collect_guards_recursive(&self, result: &mut Vec<SignalGuardInnerSync<'a>>) {
        self.mark_dirty();
        result.push(SignalGuardInnerSync(self.clone_box()));
        self.collect_and_iterate(&self.0.successors, |signal| {
            signal.collect_guards_recursive(result);
        });
    }
    fn collect_predecessors_recursive(&self, result: &mut Vec<SignalGuardInnerSync<'a>>) {
        self.mark_dirty();
        result.push(SignalGuardInnerSync(self.clone_box()));
        // Collect predecessors last so they drop last (react last)
        self.collect_and_iterate(&self.0.predecessors, |signal| {
            signal.collect_predecessors_recursive(result);
        });
    }
}

impl<T> Clone for SignalSync<'_, T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<'a, T> AsRef<SignalSync<'a, T>> for SignalSync<'a, T> {
    fn as_ref(&self) -> &SignalSync<'a, T> {
        self
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn test_signal_sync_basic() {
        let a = SignalSync::new(0);
        let _a = a.map(|x| println!("a changed: {}", x));
        (a.send(100), a.send(5));
    }

    #[test]
    fn test_signal_sync_map() {
        let a = SignalSync::new(0);
        let b = a.map(|x| x * 2);
        let _b = b.map(|x| println!("b changed: {}", x));
        drop(b);
        a.send(100);
    }

    #[test]
    fn test_signal_sync_combine() {
        let a = SignalSync::new(0);
        let b = a.map(|x| x * 2);
        let _ab = a
            .and(&b)
            .map(|(x, y)| println!("c changed: {} + {} = {}", x, y, x + y));
        (a.send(5), a.send(100));
    }

    #[test]
    fn test_signal5() {
        let result = SignalSync::new(42);
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
    fn test_depend_sync() {
        let mut a = SignalSync::new(10);
        let mut b = SignalSync::new(10);
        let c = SignalSync::new(10);

        let _observer_a = a.map(|x| println!("a changed: {}", x));
        let _observer_b = b.map(|x| println!("b changed: {}", x));
        let _observer_c = c.map(|x| println!("c changed: {}", x));

        b = c.depend(b);
        a = b.depend(a);

        (a.send(42), b.send(88));
    }
}
