use std::rc::{Rc, Weak};

pub trait LiftInto<T> {
    fn lift(self) -> T;
}

pub trait WeakLiftInto<T> {
    fn weak_lift(self) -> T;
}

/// Abstraction over strong and weak Rc references
pub enum RcRef<T> {
    Strong(Rc<T>),
    Weak(Weak<T>),
}

impl<T> RcRef<T> {
    pub fn new(rc: Rc<T>, weak: bool) -> Self {
        if weak {
            RcRef::Weak(Rc::downgrade(&rc))
        } else {
            RcRef::Strong(rc)
        }
    }
    pub fn upgrade(&self) -> Option<Rc<T>> {
        match self {
            RcRef::Strong(rc) => Some(Rc::clone(rc)),
            RcRef::Weak(weak) => weak.upgrade(),
        }
    }
}

impl<T> Clone for RcRef<T> {
    fn clone(&self) -> Self {
        match self {
            RcRef::Strong(rc) => RcRef::Strong(Rc::clone(rc)),
            RcRef::Weak(weak) => RcRef::Weak(Weak::clone(weak)),
        }
    }
}
