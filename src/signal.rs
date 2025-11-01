use std::{cell::RefCell, iter, rc::Rc};

use crate::api::Liftable;

pub(crate) trait SignalExt<'a> {
    fn get_ptr(&self) -> *const ();
    fn fire_reactions(&self);
    fn clone_box(&self) -> Box<dyn SignalExt<'a> + 'a>;
}

// Thread-local storage for batch processing
thread_local! {
    // Signals that have been explicitly modified and need processing
    // Uses HashMap to allow updates to override previous entries
    static MODIFIED_SIGNALS: RefCell<std::collections::HashMap<*const (), unsafe fn(*const ())>> = RefCell::new(std::collections::HashMap::new());
    // Order in which signals were marked (for processing in order)
    static SIGNAL_ORDER: RefCell<Vec<*const ()>> = RefCell::new(Vec::new());
    // Guards depth tracking for nested sends
    static GUARD_DEPTH: RefCell<usize> = RefCell::new(0);
}

// Mark a signal as modified in this batch
fn mark_signal_modified<'a, T: 'a>(signal: &Signal<'a, T>) {
    let ptr = Rc::as_ptr(&signal.0) as *const ();

    unsafe fn fire<T>(ptr: *const ()) {
        unsafe {
            let inner_ptr = ptr as *const SignalInner<T>;
            let inner_ref = &*inner_ptr;
            inner_ref.react_fns.borrow().iter().for_each(|f| f());
            *inner_ref.explicitly_modified.borrow_mut() = false;
        }
    }

    // Insert into map (replaces old entry if present)
    MODIFIED_SIGNALS.with(|modified| {
        modified
            .borrow_mut()
            .insert(ptr, fire::<T> as unsafe fn(*const ()));
    });

    // Update order - if already present, move to end (later modification wins)
    SIGNAL_ORDER.with(|order| {
        let mut vec = order.borrow_mut();
        // Remove from current position if present
        if let Some(pos) = vec.iter().position(|p| *p == ptr) {
            vec.remove(pos);
        }
        // Add to end
        vec.push(ptr);
    });
}

// Process all modified signals in topological order
fn process_batch() {
    // Keep processing until no more signals are marked
    loop {
        // Get the order of signals to process
        let order: Vec<*const ()> = SIGNAL_ORDER.with(|ord| std::mem::take(&mut *ord.borrow_mut()));

        if order.is_empty() {
            break;
        }

        // Fire signals in order, each with its final value from the HashMap
        for ptr in order {
            // Get and remove the fire function
            let fire_fn_opt = MODIFIED_SIGNALS.with(|modified| modified.borrow_mut().remove(&ptr));

            if let Some(fire_fn) = fire_fn_opt {
                unsafe { fire_fn(ptr) };
            }
        }
    }
}

#[allow(dead_code)]
#[allow(unused_must_use)]
pub struct SignalGuard<'a> {
    _phantom: std::marker::PhantomData<&'a ()>,
}

impl<'a> Drop for SignalGuard<'a> {
    fn drop(&mut self) {
        // Decrement depth
        let depth_after = GUARD_DEPTH.with(|d| {
            let current = *d.borrow();
            *d.borrow_mut() = current.saturating_sub(1);
            current.saturating_sub(1)
        });

        // Process batch when all guards are dropped
        if depth_after == 0 {
            process_batch();
        }
    }
}

pub struct SignalInner<'a, T> {
    pub(crate) value: RefCell<T>,
    pub(crate) react_fns: RefCell<Vec<Box<dyn Fn() + 'a>>>,
    pub(crate) successors: RefCell<Vec<Box<dyn SignalExt<'a> + 'a>>>,
    pub(crate) explicitly_modified: RefCell<bool>,
}

pub struct Signal<'a, T>(pub(crate) Rc<SignalInner<'a, T>>);

impl<'a, T: 'a> Signal<'a, T> {
    pub fn new(initial: T) -> Self {
        let inner = Rc::new(SignalInner {
            value: RefCell::new(initial),
            react_fns: RefCell::new(Vec::new()),
            successors: RefCell::new(Vec::new()),
            explicitly_modified: RefCell::new(false),
        });
        Signal(inner)
    }

    pub fn send(&self, new_value: T) -> SignalGuard<'a> {
        GUARD_DEPTH.with(|d| *d.borrow_mut() += 1);

        self.modify(|v| *v = new_value);
        *self.0.explicitly_modified.borrow_mut() = true;

        // Mark this signal as modified
        mark_signal_modified(self);

        SignalGuard {
            _phantom: std::marker::PhantomData,
        }
    }

    pub fn send_with<F>(&self, f: F) -> SignalGuard<'a>
    where
        F: FnOnce(&mut T),
    {
        GUARD_DEPTH.with(|d| *d.borrow_mut() += 1);

        self.modify(f);

        // Mark this signal as modified
        mark_signal_modified(self);

        SignalGuard {
            _phantom: std::marker::PhantomData,
        }
    }

    pub fn map<U: 'a, F>(&self, f: F) -> Signal<'a, U>
    where
        F: Fn(&T) -> U + 'a,
    {
        let new_signal = Signal::new(f(&self.0.value.borrow()));
        let cloned_new_signal = Box::new(new_signal.clone());
        let result_new_signal = new_signal.clone();
        let source_for_closure = Rc::clone(&self.0);
        let new_signal_for_mark = new_signal.clone();

        let react_fn = Box::new(move || {
            // Only update if the signal wasn't explicitly modified
            if !*new_signal.0.explicitly_modified.borrow() {
                let new_value = f(&source_for_closure.value.borrow());
                new_signal.modify(|v| *v = new_value);

                // Mark derived signal as modified so its reactions fire
                mark_signal_modified(&new_signal_for_mark);
            }
        });

        self.0.react_fns.borrow_mut().push(react_fn);
        self.0.successors.borrow_mut().push(cloned_new_signal);
        result_new_signal
    }

    pub fn comap<F, U>(&self, f: F) -> Signal<'a, U>
    where
        F: Fn(&U) -> T + 'a,
        U: Default + 'a,
    {
        let new_signal = Signal::new(U::default());
        let result_new_signal = new_signal.clone();
        let source_inner = Rc::clone(&self.0);
        let new_signal_rc = Rc::clone(&new_signal.0);

        let source_signal = Signal(Rc::clone(&source_inner));
        let react_fn = Box::new(move || {
            if *new_signal_rc.explicitly_modified.borrow() {
                let u_value_ref = new_signal_rc.value.borrow();
                let t_value = f(&u_value_ref);
                drop(u_value_ref);

                *source_inner.value.borrow_mut() = t_value;
                *source_inner.explicitly_modified.borrow_mut() = true;

                // Mark parent signal as modified for batch processing
                mark_signal_modified(&source_signal);

                *new_signal_rc.explicitly_modified.borrow_mut() = false;
            }
        });
        new_signal.0.react_fns.borrow_mut().push(react_fn);
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
        let new_signal_for_forward = new_signal.clone();
        let source_inner = Rc::clone(&self.0);
        let new_signal_rc = Rc::clone(&new_signal.0);

        // Forward reaction: T -> U (covariant)
        let new_signal_for_mark = new_signal_for_forward.clone();
        let forward_react_fn = Box::new(move || {
            if !*new_signal_rc.explicitly_modified.borrow() {
                let t_value = source_inner.value.borrow();
                let u_value = f(&t_value);
                drop(t_value);
                new_signal_for_forward.modify(|v| *v = u_value);

                // Mark derived signal as modified so its reactions fire
                mark_signal_modified(&new_signal_for_mark);
            }
        });

        self.0.react_fns.borrow_mut().push(forward_react_fn);
        self.0
            .successors
            .borrow_mut()
            .push(Box::new(new_signal.clone()));

        // Backward reaction: U -> T (contravariant)
        let source_inner_back = Rc::clone(&self.0);
        let new_signal_rc_back = Rc::clone(&new_signal.0);
        let source_signal_back = Signal(Rc::clone(&source_inner_back));

        let backward_react_fn = Box::new(move || {
            if *new_signal_rc_back.explicitly_modified.borrow() {
                let u_value_ref = new_signal_rc_back.value.borrow();
                let t_value = g(&u_value_ref);
                drop(u_value_ref);

                *source_inner_back.value.borrow_mut() = t_value;
                *source_inner_back.explicitly_modified.borrow_mut() = true;

                // Mark parent signal as modified for batch processing
                mark_signal_modified(&source_signal_back);

                *new_signal_rc_back.explicitly_modified.borrow_mut() = false;
            }
        });
        new_signal.0.react_fns.borrow_mut().push(backward_react_fn);

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

        let new_signal_for_mark_self = new_signal.clone();
        let react_fn_self = Box::new(move || {
            if !*new_signal.0.explicitly_modified.borrow() {
                new_signal.modify(|v| v.0 = source_for_closure_self.value.borrow().clone());
                mark_signal_modified(&new_signal_for_mark_self);
            }
        });
        let new_signal_for_mark_another = cloned_new_signal.clone();
        let react_fn_another = Box::new(move || {
            if !*cloned_new_signal.0.explicitly_modified.borrow() {
                cloned_new_signal
                    .modify(|v| v.1 = source_for_closure_another.value.borrow().clone());
                mark_signal_modified(&new_signal_for_mark_another);
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
                let new_signal_for_mark = new_signal.clone();

                let react_fn = Box::new(move || {
                    if !*new_signal_clone.0.explicitly_modified.borrow() {
                        new_signal_clone.modify(|v| {
                            v[index] = source_for_closure.value.borrow().clone();
                        });
                        mark_signal_modified(&new_signal_for_mark);
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
            let new_signal_for_mark = new_signal.clone();

            let react_fn = Box::new(move || {
                if !*new_signal_clone.0.explicitly_modified.borrow() {
                    new_signal_clone.modify(|v| {
                        v[index] = source_for_closure.value.borrow().clone();
                    });
                    mark_signal_modified(&new_signal_for_mark);
                }
            });

            signal.0.react_fns.borrow_mut().push(react_fn);
            signal.0.successors.borrow_mut().push(cloned_new_signal_box);
        }

        result_new_signal
    }
}

impl<'a, T: 'a> SignalExt<'a> for Signal<'a, T> {
    fn get_ptr(&self) -> *const () {
        Rc::as_ptr(&self.0) as *const ()
    }

    fn fire_reactions(&self) {
        self.0.react_fns.borrow().iter().for_each(|react_fn| {
            react_fn();
        });
        *self.0.explicitly_modified.borrow_mut() = false;
    }

    fn clone_box(&self) -> Box<dyn SignalExt<'a> + 'a> {
        Box::new(Signal(Rc::clone(&self.0)))
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
    fn test_signal1() {
        let a = Signal::new(0);
        let b = a.map(|x| x * 2);
        let _b = b.map(|x| println!("b changed: {}", x));
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

        let _ = result.map(|x| println!("result changed: {}", x));
        let _ = source1.map(|x| println!("source1 changed: {}", x));
        let _ = source2.map(|x| println!("source2 changed: {}", x));

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

        let _ = a.map(|x| println!("a changed: {}", x));
        let _ = b.map(|x| println!("b changed: {}", x));
        let _ = c.map(|x| println!("c changed: {}", x));

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
}
