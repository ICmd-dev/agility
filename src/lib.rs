use std::{
    cell::RefCell,
    rc::{Rc, Weak},
};

pub fn add(left: u64, right: u64) -> u64 {
    left + right
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}

pub trait Responsive<T = Self>
where
    T: ?Sized,
{
    fn before_change(&self);
    fn after_change(&self);
}

struct ResponsiveInner<'a, T: ?Sized> {
    successor_before_changes: Vec<Box<dyn Fn() + 'a>>,
    successor_after_changes: Vec<Box<dyn Fn() + 'a>>,
    value: T,
}

pub struct ResponsiveRef<'a, T: ?Sized> {
    inner: Rc<RefCell<ResponsiveInner<'a, T>>>,
}

impl<'a, T> ResponsiveRef<'a, T> {
    pub fn new(value: T) -> Self {
        Self {
            inner: Rc::new(RefCell::new(ResponsiveInner {
                successor_before_changes: Vec::new(),
                successor_after_changes: Vec::new(),
                value,
            })),
        }
    }
}

impl<'a, T: Responsive> ResponsiveRef<'a, T> {
    pub fn map<U, F>(&'a self, f: F) -> ResponsiveRef<'a, U>
    where
        U: Responsive + 'static,
        F: Fn(&T) -> U,
    {
        let new_value = f(&self.inner.borrow().value);
        let new_rc = ResponsiveRef::new(new_value);
        let new_weak = Rc::downgrade(&new_rc.inner);
        self.inner
            .borrow_mut()
            .successor_before_changes
            .push(Box::new(move || {
                if let Some(new_strong) = new_weak.upgrade() {
                    new_strong.borrow().value.before_change();
                }
            }));
        let new_weak = Rc::downgrade(&new_rc.inner);
        self.inner
            .borrow_mut()
            .successor_after_changes
            .push(Box::new(move || {
                if let Some(new_strong) = new_weak.upgrade() {
                    new_strong.borrow().value.after_change();
                }
            }));
        new_rc
    }
    pub fn set(&self, new_value: T) {
        let inner_ref = self.inner.borrow();
        inner_ref.value.before_change();
        for successor_before_change in &inner_ref.successor_before_changes {
            successor_before_change();
        }
        self.inner.borrow_mut().value = new_value;
    }
}

impl<T: Responsive + ?Sized> Clone for ResponsiveRef<'_, T> {
    fn clone(&self) -> Self {
        Self {
            inner: Rc::clone(&self.inner),
        }
    }
}
