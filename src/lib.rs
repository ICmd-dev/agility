pub mod api;
pub mod concurrent;
pub mod signal;

pub use agility_macros::*;

pub fn add(left: u64, right: u64) -> u64 {
    left + right
}

#[cfg(test)]
mod tests {
    use crate::{
        api::{self, LiftInto},
        lift_struct,
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
        (signal_r, signal_l)
            .lift()
            .map(|(r, l)| println!("({}, {})", r, l));
        println!("Sending 7 and 8...");
        (signal.send(7), signal.send(8));
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
        let c = [a, b].lift();
        c.map(|s| s.iter().map(|x| *x).collect::<Vec<_>>())
            .map(|v| println!("c: {:?}", v));
        println!("Sending 7 and 8...");
        (entrance.send(7), entrance.send(8));
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

    #[lift_struct]
    struct TestStruct {
        #[signal]
        a: i32,
        #[signal]
        b: i32,
        c: String,
    }

    #[test]
    fn lift_struct() {
        use api::LiftInto;

        let entrance = Signal::new(5);
        let a = entrance.map(|x| {
            let new = x + 1;
            println!("a: {}", new);
            new
        });
        let b = entrance.map(|x| {
            let new = x * 2;
            println!("b: {}", new);
            new
        });

        let test_struct = TestStruct {
            a,
            b,
            c: "test".to_string(),
        };

        let lifted = test_struct.lift();
        lifted.map(|s| {
            println!("lifted: a = {}, b = {}, c = {}", s.a, s.b, s.c);
        });

        entrance.send(10);
        entrance.send(20);
    }
}
