pub mod api;
//pub mod concurrent;
pub mod signal;
pub mod signal_sync;
// pub mod signals;

pub use agility_macros::*;
pub use signal::*;

#[test]
fn it_works() {
    #[derive(Lift)]
    struct Point<'a> {
        x: Signal<'a, i32>,
        y: Signal<'a, i32>,
    }

    let p = Point {
        x: Signal::new(1),
        y: Signal::new(2),
    };

    let lifted = p.lift();
    lifted.with(|pt| {
        println!("Point: ({}, {})", pt.x, pt.y);
    });

    (
        lifted.send_with(|p| p.x = 10),
        lifted.send_with(|p| p.y = 20),
    );
}
