pub trait LiftInto<T> {
    fn lift(self) -> T;
}

pub trait WeakLiftInto<T> {
    fn weak_lift(self) -> T;
}
