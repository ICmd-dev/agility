use std::{
    cell::{OnceCell, RefCell},
    rc::Rc,
};

pub struct SignalGuard;

trait SignalExt<'a> {
    fn react(&self);
}

pub struct SignalInner<'a, T> {
    value: RefCell<T>,
    react_fn: OnceCell<Box<dyn Fn() -> T + 'a>>,
    successors: RefCell<Vec<Box<dyn SignalExt<'a> + 'a>>>,
}

pub struct Signal<'a, T>(Rc<SignalInner<'a, T>>);

impl<'a, T: 'a> Signal<'a, T> {
    pub fn new(initial: T) -> Self {
        let inner = Rc::new(SignalInner {
            value: RefCell::new(initial),
            react_fn: OnceCell::new(),
            successors: RefCell::new(Vec::new()),
        });
        Signal(inner)
    }

    pub fn send(&self, new_value: T) {
        self.modify(|v| *v = new_value);
        self.propagate();
    }

    pub fn send_with<F>(&self, f: F)
    where
        F: FnOnce(&mut T),
    {
        self.modify(f);
        self.propagate();
    }

    pub fn map<U: 'a, F>(&self, f: F) -> Signal<'a, U>
    where
        F: Fn(&T) -> U + 'a,
    {
        let new_signal = Signal::new(f(&self.0.value.borrow()));
        let source_for_closure = Rc::clone(&self.0);
        let _ = new_signal
            .0
            .react_fn
            .set(Box::new(move || f(&source_for_closure.value.borrow())));

        let cloned_new_signal: Box<dyn SignalExt<'a>> = Box::new(Signal(Rc::clone(&new_signal.0)));
        self.0.successors.borrow_mut().push(cloned_new_signal);
        new_signal
    }

    #[inline]
    fn modify(&self, f: impl FnOnce(&mut T)) {
        let mut value = self.0.value.borrow_mut();
        f(&mut value);
    }

    #[inline]
    fn propagate(&self) {
        for successor in self.0.successors.borrow().iter() {
            successor.react();
        }
    }
}

impl<'a, T: 'a> SignalExt<'a> for Signal<'a, T> {
    fn react(&self) {
        let Some(react_fn) = self.0.react_fn.get() else {
            return;
        };
        let mut value = self.0.value.borrow_mut();
        let new_value = react_fn();
        *value = new_value;
        self.propagate();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_signal() {
        let signal = Signal::new(0);
        let _a = signal.map(|i| {
            println!("Mapping signal value: {}", *i);
            *i + 1
        });
        (signal.send(5), signal.send(10));
    }
}
