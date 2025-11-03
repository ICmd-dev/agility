pub mod api;
pub mod concurrent;
pub mod signal;
pub mod signal_sync;
pub mod signals;

pub use agility_macros::*;

pub fn add(left: u64, right: u64) -> u64 {
    left + right
}

#[cfg(test)]
mod tests {
    use agility_macros::LiftSync;

    use crate::Lift;
    use crate::signal::Signal;
    use crate::signal_sync::SignalSync;

    #[test]
    fn test_lift_macro() {
        #[derive(Lift)]
        struct A<'a> {
            a: Signal<'a, i32>,
            b: Signal<'a, bool>,
            s: String,
        }

        let instance = A {
            a: Signal::new(42),
            b: Signal::new(true),
            s: "hello".to_string(),
        };

        let lifted = instance.lift();
        let _observer = lifted.map(|inner| {
            println!("a: {}, b: {}, s: {}", inner.a, inner.b, inner.s);
        });
    }

    #[test]
    fn test_lift_macro_reactivity() {
        #[derive(Clone, LiftSync)]
        struct Point<'a, A: 'a + Send + Sync> {
            x: SignalSync<'a, i32>,
            y: SignalSync<'a, i32>,
            label: A,
        }

        let point = Point {
            x: SignalSync::new(10),
            y: SignalSync::new(20),
            label: "Origin",
        };

        // From Point to Signal<_Point>
        let lifted = point.clone().lift();

        let _observer = lifted.map(|inner| {
            println!("Point {}: ({}, {})", inner.label, inner.x, inner.y);
        });

        point.x.send(12);
        point.y.send(25);

        lifted.send(_Point {
            x: 50,
            y: 75,
            label: "Modified",
        });
    }
}
