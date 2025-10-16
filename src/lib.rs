pub mod api;
pub mod concurrent;
pub mod signal;

pub fn add(left: u64, right: u64) -> u64 {
    left + right
}

#[cfg(test)]
mod tests {
    use crate::{api::LiftInto, signal::Signal};

    #[test]
    fn it_works() {
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
        println!("Sending 7...");
        signal.send(7);
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
}
