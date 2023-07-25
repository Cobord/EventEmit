mod events;

use std::{
    cmp::max,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use crate::events::{Emitter, Interleaves};

#[derive(PartialEq, Eq, Hash, Clone)]
#[repr(transparent)]
struct AB(bool);
impl<T> Interleaves<T> for AB {
    fn do_interleave(&self, _: &T, other: &Self, _: &T) -> bool {
        self == other
    }

    fn interleaves_with_everything(&self, _my_aux: &T) -> bool {
        false
    }
}
use std::fmt::{Debug, Error, Formatter};
impl Debug for AB {
    fn fmt(&self, f: &mut Formatter) -> Result<(), Error> {
        write!(f, "{:?}", self.0)
    }
}

fn common_resource() {
    use std::sync::{Arc, Mutex};

    let a = |(other_operand, wait_millis, data1): (i32, u64, Arc<Mutex<i32>>)| {
        thread::sleep(Duration::from_millis(wait_millis));
        let mut data = data1.lock().unwrap();
        println!("ping ");
        *data += other_operand;
    };

    let b = |(other_operand, wait_millis, data2): (i32, u64, Arc<Mutex<i32>>)| {
        thread::sleep(Duration::from_millis(wait_millis));
        let mut data = data2.lock().unwrap();
        println!("pong ");
        *data *= other_operand;
    };

    let mut emitter: Emitter<AB, (i32, u64, Arc<Mutex<i32>>), ()> = Emitter::new(None);
    let true_new = emitter.on(AB(true), a);
    assert!(true_new);
    let false_new = emitter.on(AB(false), b);
    assert!(false_new);

    let mut exp_data_val = 1;
    let data = Arc::new(Mutex::new(exp_data_val));

    let mut other_operand;
    let mut expected_time = Duration::from_millis(0);
    let mut serial_time = Duration::from_millis(0);

    other_operand = 1;
    let wait_time_1 = 50;
    emitter.emit(AB(true), (other_operand, wait_time_1, data.clone()));
    exp_data_val += other_operand;

    other_operand = 10;
    let wait_time_2 = 60;
    emitter.emit(AB(true), (other_operand, wait_time_2, data.clone()));
    exp_data_val += other_operand;

    expected_time += Duration::from_millis(max(wait_time_1, wait_time_2));
    serial_time += Duration::from_millis(wait_time_1);
    serial_time += Duration::from_millis(wait_time_2);

    other_operand = 2;
    let wait_time_3 = 50;
    emitter.emit(AB(false), (other_operand, wait_time_3, data.clone()));
    exp_data_val *= other_operand;
    expected_time += Duration::from_millis(wait_time_3);
    serial_time += Duration::from_millis(wait_time_3);

    other_operand = 24;
    let wait_time_4 = 20;
    emitter.emit(AB(true), (other_operand, wait_time_4, data.clone()));
    exp_data_val += other_operand;
    expected_time += Duration::from_millis(wait_time_4);
    serial_time += Duration::from_millis(wait_time_4);

    let loop_time = Duration::from_millis(15);
    let now = Instant::now();
    emitter.wait_for_all(loop_time);
    let elapsed = now.elapsed();
    println!(
        "{} milliseconds vs {} with no waiting and {} serially",
        elapsed.as_millis(),
        expected_time.as_millis(),
        serial_time.as_millis()
    );
    assert!(elapsed < expected_time + 3 * loop_time);
    assert!(elapsed < serial_time);
    let data_val = *data.lock().unwrap();
    assert_eq!(data_val, exp_data_val);
}

fn main() {
    env_logger::init();

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

    common_resource();
}
