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
    use std::thread;
    use std::time::Duration;

    use agility_macros::LiftSync;

    use crate::Lift;
    use crate::signal::Signal;
    use crate::signal_sync::SignalSync;
    use crate::signals::clock_tick;

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

    #[test]
    fn test_listener() {
        let clock_signal = clock_tick(Duration::from_millis(500), Duration::from_secs(10));
        let clock_signal2 = clock_tick(Duration::from_millis(250), Duration::from_secs(5));

        let combined_signal = clock_signal.combine(&clock_signal2);
        combined_signal.with(|(tick1, tick2)| {
            println!("Tick1:\t{}, Tick2:\t{}", tick1, tick2);
        });

        thread::sleep(Duration::from_secs(12));
    }
}
