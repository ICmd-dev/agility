use crate::signal::Signal;

trait Mutable {}

pub trait LiftInto<T> {
    fn lift(self) -> T;
}

pub trait Liftable<'a> {
    type Inner;
    fn as_ref(&self) -> &Signal<'a, Self::Inner>;
}

impl<T> Mutable for Vec<T> {}

impl<'a, T> Liftable<'a> for Signal<'a, T> {
    type Inner = T;
    fn as_ref(&self) -> &Signal<'a, Self::Inner> {
        &self
    }
}

impl<'a, T> Liftable<'a> for &Signal<'a, T> {
    type Inner = T;
    fn as_ref(&self) -> &Signal<'a, Self::Inner> {
        self
    }
}

impl<'a, S1, S2> LiftInto<Signal<'a, (S1::Inner, S2::Inner)>> for (S1, S2)
where
    S1: Liftable<'a>,
    S2: Liftable<'a>,
    S1::Inner: Clone + 'a,
    S2::Inner: Clone + 'a,
{
    fn lift(self) -> Signal<'a, (S1::Inner, S2::Inner)> {
        self.0.as_ref().with(self.1.as_ref())
    }
}

impl<'a, S, I> LiftInto<Signal<'a, Vec<S::Inner>>> for I
where
    S: Liftable<'a>,
    I: IntoIterator<Item = S> + Mutable,
    S::Inner: Clone + 'a,
{
    fn lift(self) -> Signal<'a, Vec<S::Inner>> {
        let mut items: Vec<S> = self.into_iter().collect();
        if items.is_empty() {
            Signal::new(Vec::new())
        } else {
            let first = items.remove(0);
            first.as_ref().extend(items.into_iter())
        }
    }
}

impl<'a, const N: usize, S> LiftInto<Signal<'a, [S::Inner; N]>> for [S; N]
where
    S: Liftable<'a>,
    S::Inner: Clone + 'a,
{
    fn lift(self) -> Signal<'a, [S::Inner; N]> {
        Signal::<S::Inner>::lift_from_array::<S, N>(self)
    }
}
