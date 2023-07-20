mod events;

use std::{sync::mpsc, thread, time::Duration};

use crate::events::{Commuting, Emitter};

fn main() {
    env_logger::init();

    #[derive(PartialEq, Eq, Hash, Clone)]
    #[repr(transparent)]
    struct AB(bool);
    impl Commuting<u64> for AB {
        fn do_commute(&self, _: &u64, other: &Self, _: &u64) -> bool {
            self == other
        }

        fn commutes_with_everything(&self, _my_aux: &u64) -> bool {
            false
        }
    }
    use std::fmt::{Debug, Error, Formatter};
    impl Debug for AB {
        fn fmt(&self, f: &mut Formatter) -> Result<(), Error> {
            write!(f, "{:?}", self.0)
        }
    }

    let (tx, rx) = mpsc::channel();
    let mut emitter: Emitter<AB, u64, ()> = Emitter::new(Some(tx.clone()));
    let mut emitter2: Emitter<AB, u64, ()> = Emitter::new(Some(tx));
    let true_new = emitter.on(AB(true), |wait_time| {
        thread::sleep(Duration::from_secs(wait_time));
        println!("True")
    });
    assert!(true_new);
    let false_new = emitter.on(AB(false), |wait_time| {
        thread::sleep(Duration::from_secs(wait_time));
        println!("False")
    });
    assert!(false_new);
    let true_new = emitter2.on(AB(true), |wait_time| {
        thread::sleep(Duration::from_secs(wait_time));
        println!("True from 2")
    });
    assert!(true_new);

    let mut total_emissions = 0;

    let (exists, spawn_later, spawned) = emitter.emit(AB(true), 3);
    total_emissions += 1;
    assert!(exists && spawned && !spawn_later);

    let (exists, spawn_later, spawned) = emitter.emit(AB(true), 2);
    total_emissions += 1;
    assert!(exists && spawned && !spawn_later);

    let (exists, spawn_later, spawned) = emitter.emit(AB(false), 1);
    total_emissions += 1;
    assert!(exists && !spawned && spawn_later);

    let true_used_to_exist = emitter.off(AB(true));
    assert!(true_used_to_exist);

    let (exists, spawn_later, spawned) = emitter.emit(AB(true), 10);
    assert!(!exists && !spawn_later && !spawned);

    let (exists, spawn_later, spawned) = emitter2.emit(AB(true), 1);
    total_emissions += 1;
    assert!(exists && spawned && !spawn_later);

    let j1 = thread::spawn(move || {
        emitter.wait_for_all(Duration::new(1, 0));
    });
    let j2 = thread::spawn(move || {
        emitter2.wait_for_all(Duration::new(1, 0));
    });
    assert!(j2.join().is_ok());
    assert!(j1.join().is_ok());
    for v in rx.iter().take(total_emissions) {
        println!(
            "Emission {} with payload emit({:?},{:?}) with result {:?} : received on the channel",
            v.0, v.1, v.2, v.3
        );
    }
    let final_wait = Duration::new(5, 0);
    println!(
        "Waiting for {:?} to make sure nothing comes down the channel after wait for all",
        final_wait
    );
    let next_item = rx.recv_timeout(final_wait);
    assert!(next_item.is_err());
}
