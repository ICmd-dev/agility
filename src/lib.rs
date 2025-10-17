pub mod api;
pub mod concurrent;
pub mod signal;
pub mod signal_v2;

pub use agility_macros::*;

pub fn add(left: u64, right: u64) -> u64 {
    left + right
}

#[cfg(test)]
mod tests {
    use agility_macros::lift;

    use crate::{
        api::{self, LiftInto},
        signal::{self, Signal},
    };

    #[test]
    fn it_works() {
        let length = Signal::new(0);
        let array: Signal<Box<[i32]>> = length.map(|l| vec![6; *l].into_boxed_slice());
        array.map(|a| println!("Array: {:?}", a));
        length.send(3);
        length.send(4);
    }

    #[test]
    fn combined() {
        let signal = Signal::new(5);
        let signal_r = signal.map(|x| {
            let new = x + 1;
            println!("signal_r: {}", new);
            new
        });
        let signal_l = signal.map(|x| {
            let new = x * 2;
            println!("signal_l: {}", new);
            new
        });
        (signal_r.clone_refered(), signal_l.clone_refered())
            .lift()
            .map(|(r, l)| println!("({}, {})", r, l));
        println!("Sending 7 and 8...");
        signal_r.send(7);
        signal_l.send(8);
        println!("Sending 10 and 11... Together");
        (signal_r.send(10), signal_l.send(11));
        println!("Sending 666 hrough original signal");
        signal.send(666);
        println!("Done.");
    }

    #[test]
    fn sequence() {
        let entrance = Signal::new(5);
        let a = entrance.map(|x| {
            let new = x + 1;
            println!("a: {}", new);
            new
        });
        let b = a.map(|x| {
            let new = x * 2;
            println!("b: {}", new);
            new
        });
        let c = [a.clone_refered(), b].lift();
        c.map(|s| s.iter().map(|x| *x).collect::<Vec<_>>())
            .map(|v| println!("c: {:?}", v));
        println!("Sending 7 and 8...");
        a.send(12);
    }

    #[test]
    fn map() {
        let entrance = Signal::new(5);
        let a = entrance.map(|x| {
            let new = x + 1;
            println!("a: {}", new);
            new
        });
        let b = a.map(|x| {
            let new = x * 2;
            println!("b: {}", new);
            new
        });
        let c = entrance.map(|x| {
            let new = x * x;
            println!("c: {}", new);
            new
        });
        let d = c.map(|x| {
            let new = x - 3;
            println!("d: {}", new);
            new
        });
        println!("Sending 7, 8, and 12...");
        (entrance.send(7), entrance.send(8), entrance.send(12));
    }

    #[test]
    fn draw() {}

    #[lift]
    #[derive(Debug)]
    struct Position {
        #[signal]
        pub x: f32,
        #[signal]
        pub y: f32,
        #[signal]
        pub z: f32,
    }

    #[test]
    fn lift_struct() {
        use api::LiftInto;

        let entrance = Signal::new(5.0f32);
        let x = entrance.map(|v| *v * 0.1);
        let y = entrance.map(|v| *v + 0.2);
        let z = entrance.map(|v| *v * *v);
        let position = Position {
            x: x.clone_refered(),
            y: y.clone_refered(),
            z: z.clone_refered(),
        }
        .lift();
        position.map(|p| {
            println!("{:?}", p);
        });

        x.send(12.0);
    }
}
