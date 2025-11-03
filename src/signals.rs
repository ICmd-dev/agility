use std::{thread, time::Duration};

use crate::signal_sync::SignalSync;

pub fn clock_tick(gap: Duration, total: Duration) -> SignalSync<'static, u64> {
    let signal = SignalSync::new(0u64);
    let signal_clone = signal.clone();
    thread::spawn(move || {
        let mut elapsed = Duration::from_secs(0);
        let mut ticks = 0;
        while elapsed < total {
            thread::sleep(gap);
            elapsed += gap;
            signal_clone.send(ticks);
            ticks += 1;
        }
    });
    signal
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clock_tick() {
        let tick_signal = clock_tick(Duration::from_millis(100), Duration::from_secs(1));
        let observer = tick_signal.map(|tick| {
            println!("Tick: {}", tick);
        });

        thread::sleep(Duration::from_secs(2));
        drop(observer);
    }
}
