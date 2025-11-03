use std::{
    iter,
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicBool, AtomicIsize, Ordering},
    },
};

use crate::api::LiftableSync;

pub trait SignalExtSync<'a> {
    fn react(&self);
    fn guard(&self) -> SignalGuardSync<'a>;
    fn decrease_dirty(&self);
    fn get_dirty(&self) -> isize;
    fn clone_box(&self) -> Box<dyn SignalExtSync<'a> + 'a>;
    fn collect_guards_recursive(&self, result: &mut Vec<SignalGuardInnerSync<'a>>);
    fn collect_predecessors_recursive(&self, result: &mut Vec<SignalGuardInnerSync<'a>>);
    fn reset_explicitly_modified(&self);
}

// Helper struct to hold weak references that can be upgraded (thread-safe version)
pub struct WeakSignalRefSync<'a> {
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

pub struct SignalGuardInnerSync<'a>(Box<dyn SignalExtSync<'a> + 'a>);

#[allow(dead_code)]
#[allow(unused_must_use)]
pub struct SignalGuardSync<'a>(Vec<SignalGuardInnerSync<'a>>);

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

pub struct SignalInnerSync<'a, T> {
    pub(crate) value: Mutex<T>,
    pub(crate) react_fns: RwLock<Vec<Box<dyn Fn() + Send + Sync + 'a>>>,
    pub(crate) successors: RwLock<Vec<WeakSignalRefSync<'a>>>,
    pub(crate) predecessors: RwLock<Vec<WeakSignalRefSync<'a>>>,
    pub(crate) dirty: AtomicIsize,
    pub(crate) explicitly_modified: AtomicBool,
}

pub struct SignalSync<'a, T>(pub(crate) Arc<SignalInnerSync<'a, T>>);

impl<'a, T: Send + Sync + 'a> SignalSync<'a, T> {
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

    pub fn send(&self, new_value: T) -> SignalGuardSync<'a> {
        self.modify(|v| *v = new_value);
        self.0.explicitly_modified.store(true, Ordering::Release);
        self.guard()
    }

    pub fn send_with<F>(&self, f: F) -> SignalGuardSync<'a>
    where
        F: FnOnce(&mut T),
    {
        self.modify(f);
        self.guard()
    }

    pub fn map<U: Send + Sync + 'a, F>(&self, f: F) -> SignalSync<'a, U>
    where
        F: Fn(&T) -> U + Send + Sync + 'a,
    {
        let new_signal = SignalSync::new(f(&self.0.value.lock().unwrap()));
        let result_new_signal = new_signal.clone();
        let new_signal_for_react = Arc::downgrade(&new_signal.0);
        let source_for_closure = Arc::downgrade(&self.0);

        let react_fn = Box::new(move || {
            // Only update if the signal wasn't explicitly modified
            if let Some(new_sig) = new_signal_for_react.upgrade() {
                if !new_sig.explicitly_modified.load(Ordering::Acquire) {
                    if let Some(source) = source_for_closure.upgrade() {
                        let new_value = f(&source.value.lock().unwrap());
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

    pub fn comap<F, U>(&self, f: F) -> SignalSync<'a, U>
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

    pub fn with<S>(&self, another: S) -> SignalSync<'a, (T, S::Inner)>
    where
        S: LiftableSync<'a>,
        S::Inner: Clone + Send + Sync + 'a,
        T: Clone,
    {
        let another = another.as_ref();
        let new_signal = SignalSync::new((
            self.0.value.lock().unwrap().clone(),
            another.0.value.lock().unwrap().clone(),
        ));
        let result_new_signal = new_signal.clone();
        let new_signal_weak = Arc::downgrade(&new_signal.0);
        let source_for_closure_self = Arc::downgrade(&self.0);
        let source_for_closure_another = Arc::downgrade(&another.0);

        let react_fn_self = Box::new(move || {
            if let Some(new_sig) = new_signal_weak.upgrade() {
                if !new_sig.explicitly_modified.load(Ordering::Acquire) {
                    if let Some(source) = source_for_closure_self.upgrade() {
                        new_sig.value.lock().unwrap().0 = source.value.lock().unwrap().clone();
                    }
                }
            }
        });
        let new_signal_weak_2 = Arc::downgrade(&new_signal.0);
        let react_fn_another = Box::new(move || {
            if let Some(new_sig) = new_signal_weak_2.upgrade() {
                if !new_sig.explicitly_modified.load(Ordering::Acquire) {
                    if let Some(source) = source_for_closure_another.upgrade() {
                        new_sig.value.lock().unwrap().1 = source.value.lock().unwrap().clone();
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

    pub fn extend<S>(&self, others: impl IntoIterator<Item = S>) -> SignalSync<'a, Vec<T>>
    where
        S: LiftableSync<'a, Inner = T>,
        T: Clone,
    {
        let others_signals: Vec<SignalSync<'a, T>> =
            others.into_iter().map(|s| s.as_ref().clone()).collect();

        let new_signal: SignalSync<'a, Vec<T>> = SignalSync::new(
            iter::once(self)
                .chain(others_signals.iter())
                .map(|s| s.0.value.lock().unwrap().clone())
                .collect(),
        );
        let result_new_signal = new_signal.clone();

        iter::once(self)
            .chain(others_signals.iter())
            .enumerate()
            .for_each(|(index, signal)| {
                let new_signal_weak = Arc::downgrade(&new_signal.0);
                let source_for_closure = Arc::downgrade(&signal.0);

                let react_fn = Box::new(move || {
                    if let Some(new_sig) = new_signal_weak.upgrade() {
                        if !new_sig.explicitly_modified.load(Ordering::Acquire) {
                            if let Some(source) = source_for_closure.upgrade() {
                                new_sig.value.lock().unwrap()[index] =
                                    source.value.lock().unwrap().clone();
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
        // Collect live references and execute callbacks within the read lock scope
        let signals_to_process: Vec<_> = {
            let mut refs_write = refs.write().unwrap();
            refs_write.retain(|s| s.is_alive());
            // Clone references while holding write lock to avoid iterator invalidation
            refs_write.iter().filter_map(|s| s.upgrade()).collect()
        };

        // Execute callbacks outside of any lock
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

    pub fn lift_from_array<S, const N: usize>(items: [S; N]) -> SignalSync<'a, [S::Inner; N]>
    where
        S: LiftableSync<'a>,
        S::Inner: Clone + Send + Sync + 'a,
    {
        let signals: [SignalSync<'a, S::Inner>; N] =
            std::array::from_fn(|i| items[i].as_ref().clone());

        let initial: [S::Inner; N] =
            std::array::from_fn(|i| signals[i].0.value.lock().unwrap().clone());

        let new_signal: SignalSync<'a, [S::Inner; N]> = SignalSync::new(initial);
        let result_new_signal = new_signal.clone();

        for (index, signal) in signals.iter().enumerate() {
            let new_signal_weak = Arc::downgrade(&new_signal.0);
            let source_for_closure = Arc::downgrade(&signal.0);

            let react_fn = Box::new(move || {
                if let Some(new_sig) = new_signal_weak.upgrade() {
                    if !new_sig.explicitly_modified.load(Ordering::Acquire) {
                        if let Some(source) = source_for_closure.upgrade() {
                            new_sig.value.lock().unwrap()[index] =
                                source.value.lock().unwrap().clone();
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
    fn test_signal_sync_with() {
        let a = SignalSync::new(0);
        let b = a.map(|x| x * 2);
        let ab = a.with(&b);
        let _ab = ab.map(|(x, y)| println!("c changed: {} + {} = {}", x, y, x + y));
        (a.send(5), a.send(100));
    }

    #[test]
    fn test_signal5() {
        let result = SignalSync::new(42);
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
}
