use crate::signal::Signal;
use crate::signal_sync::SignalSync;

trait Mutable {}

pub trait LiftInto<T> {
    fn lift(self) -> T;
}

pub trait LiftIntoSync<T> {
    fn lift(self) -> T;
}

pub trait Liftable<'a> {
    type Inner;
    fn as_ref(&self) -> &Signal<'a, Self::Inner>;
}

pub trait LiftableSync<'a> {
    type Inner;
    fn as_ref(&self) -> &SignalSync<'a, Self::Inner>;
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

impl<'a, T: Send + Sync + 'a> LiftableSync<'a> for SignalSync<'a, T> {
    type Inner = T;
    fn as_ref(&self) -> &SignalSync<'a, Self::Inner> {
        &self
    }
}

impl<'a, T: Send + Sync + 'a> LiftableSync<'a> for &SignalSync<'a, T> {
    type Inner = T;
    fn as_ref(&self) -> &SignalSync<'a, Self::Inner> {
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

// LiftIntoSync implementations for thread-safe variant
impl<'a, S1, S2> LiftIntoSync<SignalSync<'a, (S1::Inner, S2::Inner)>> for (S1, S2)
where
    S1: LiftableSync<'a>,
    S2: LiftableSync<'a>,
    S1::Inner: Clone + Send + Sync + 'a,
    S2::Inner: Clone + Send + Sync + 'a,
{
    fn lift(self) -> SignalSync<'a, (S1::Inner, S2::Inner)> {
        self.0.as_ref().with(self.1.as_ref())
    }
}

impl<'a, S, I> LiftIntoSync<SignalSync<'a, Vec<S::Inner>>> for I
where
    S: LiftableSync<'a>,
    I: IntoIterator<Item = S> + Mutable,
    S::Inner: Clone + Send + Sync + 'a,
{
    fn lift(self) -> SignalSync<'a, Vec<S::Inner>> {
        let mut items: Vec<S> = self.into_iter().collect();
        if items.is_empty() {
            SignalSync::new(Vec::new())
        } else {
            let first = items.remove(0);
            first.as_ref().extend(items.into_iter())
        }
    }
}

impl<'a, const N: usize, S> LiftIntoSync<SignalSync<'a, [S::Inner; N]>> for [S; N]
where
    S: LiftableSync<'a>,
    S::Inner: Clone + Send + Sync + 'a,
{
    fn lift(self) -> SignalSync<'a, [S::Inner; N]> {
        SignalSync::<S::Inner>::lift_from_array::<S, N>(self)
    }
}
