pub mod api;
pub mod signal;

pub fn add(left: u64, right: u64) -> u64 {
    left + right
}

#[cfg(test)]
mod tests {
    use std::{thread, time::Duration};

    use crate::signal::{LiftInto, Signal};

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
}
