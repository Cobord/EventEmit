mod events;
mod general_emitter;
mod interleaving;
mod tokio_events;
mod utils;

use std::{
    cmp::max,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use crate::general_emitter::{GeneralEmitter, SyncEmitter};
use crate::interleaving::Interleaves;
use crate::{events::SpecificThreadEmitter, tokio_events::SpecificTokioEmitter};

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

fn common_resource_1() {
    use std::sync::{Arc, Mutex};

    let a = |(other_operand, wait_millis, data1): (i32, u64, Arc<Mutex<i32>>)| {
        thread::sleep(Duration::from_millis(wait_millis));
        let mut data = data1.lock().expect("Acquiring lock here shouldn't panic");
        println!("ping ");
        *data += other_operand;
    };

    let b = |(other_operand, wait_millis, data2): (i32, u64, Arc<Mutex<i32>>)| {
        thread::sleep(Duration::from_millis(wait_millis));
        let mut data = data2.lock().expect("Acquiring lock here shouldn't panic");
        println!("pong ");
        *data *= other_operand;
    };

    let identity = |x| x;
    type MyArgType = (i32, u64, Arc<Mutex<i32>>);
    let mut emitter: SpecificThreadEmitter<AB, MyArgType, (), _> =
        SpecificThreadEmitter::new(None, identity);
    let true_new = emitter.on_sync(AB(true), a);
    assert_ok_equal!(true_new, true, "Should be no problem turning on");
    let false_new = emitter.on_sync(AB(false), b);
    assert_ok_equal!(false_new, true, "Should be no problem turning on");

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
    let data_val = *data.lock().expect("Acquiring lock here shouldn't panic");
    assert_eq!(data_val, exp_data_val);
}

#[allow(dead_code)]
fn common_resource_2() {
    use std::sync::{Arc, Mutex};

    fn a((other_operand, wait_millis, data1): (i32, u64, Arc<Mutex<i32>>)) {
        thread::sleep(Duration::from_millis(wait_millis));
        let mut data = data1.lock().expect("Acquiring lock here shouldn't panic");
        println!("ping ");
        *data += other_operand;
    }

    fn b((other_operand, wait_millis, data2): (i32, u64, Arc<Mutex<i32>>)) {
        thread::sleep(Duration::from_millis(wait_millis));
        let mut data = data2.lock().expect("Acquiring lock here shouldn't panic");
        println!("pong ");
        *data *= other_operand;
    }

    let identity = |x| x;
    type MyArgType = (i32, u64, Arc<Mutex<i32>>);
    let (tx, rx) = mpsc::channel();
    let mut emitter: SpecificTokioEmitter<AB, MyArgType, (), _> =
        SpecificTokioEmitter::new(Some(tx.clone()), identity);
    let true_new = emitter.on_sync(AB(true), a);
    assert_ok_equal!(true_new, true, "Should be no problem turning on");
    let false_new = emitter.on_sync(AB(false), b);
    assert_ok_equal!(false_new, true, "Should be no problem turning on");

    let mut exp_data_val = 1;
    let data = Arc::new(Mutex::new(exp_data_val));

    let mut other_operand;
    let mut expected_time = Duration::from_millis(0);
    let mut serial_time = Duration::from_millis(0);
    let mut total_operations = 0;

    other_operand = 1;
    let wait_time_1 = 50;
    emitter.emit(AB(true), (other_operand, wait_time_1, data.clone()));
    exp_data_val += other_operand;
    total_operations += 1;
    assert_eq!(
        emitter.count_running() + emitter.count_waiting(),
        total_operations
    );

    other_operand = 10;
    let wait_time_2 = 60;
    emitter.emit(AB(true), (other_operand, wait_time_2, data.clone()));
    exp_data_val += other_operand;
    total_operations += 1;
    assert_eq!(
        emitter.count_running() + emitter.count_waiting(),
        total_operations
    );

    expected_time += Duration::from_millis(max(wait_time_1, wait_time_2));
    serial_time += Duration::from_millis(wait_time_1);
    serial_time += Duration::from_millis(wait_time_2);

    other_operand = 2;
    let wait_time_3 = 50;
    emitter.emit(AB(false), (other_operand, wait_time_3, data.clone()));
    exp_data_val *= other_operand;
    expected_time += Duration::from_millis(wait_time_3);
    serial_time += Duration::from_millis(wait_time_3);
    total_operations += 1;
    assert_eq!(
        emitter.count_running() + emitter.count_waiting(),
        total_operations
    );

    other_operand = 24;
    let wait_time_4 = 20;
    emitter.emit(AB(true), (other_operand, wait_time_4, data.clone()));
    exp_data_val += other_operand;
    expected_time += Duration::from_millis(wait_time_4);
    serial_time += Duration::from_millis(wait_time_4);
    total_operations += 1;
    assert_eq!(
        emitter.count_running() + emitter.count_waiting(),
        total_operations
    );

    let loop_time = Duration::from_millis(15);
    let now = Instant::now();
    let _ = tx.send((0, AB(true), (1, 2, data.clone()), ()));
    for v in rx.iter().take(total_operations) {
        println!(
            "Emission {:?} with payload emit({:?},operand={:?},sleeping={:?},&mut data) with result {:?} : received on the channel",
            v.0, v.1, v.2.0, v.2.1, v.3
        );
    }
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
    let data_val = *data.lock().expect("Acquiring lock here shouldn't panic");
    assert_eq!(data_val, exp_data_val);

    let final_wait = Duration::new(1, 0);
    let next_item = rx
        .recv_timeout(final_wait)
        .map(|(a, b, c, _)| (a, b, c.0, c.1));
    assert_eq!(next_item, Err(std::sync::mpsc::RecvTimeoutError::Timeout));
}

fn main_part1() {
    let (tx, rx) = mpsc::channel();
    let identity = |x| x;
    let mut emitter: SpecificThreadEmitter<AB, u64, (), _> =
        SpecificThreadEmitter::new(Some(tx.clone()), identity);
    let mut emitter2: SpecificThreadEmitter<AB, u64, (), _> =
        SpecificThreadEmitter::new(Some(tx), identity);
    let true_new = emitter.on_sync(AB(true), |wait_time| {
        thread::sleep(Duration::from_millis(wait_time * 100));
        println!("True");
    });
    assert_ok_equal!(true_new, true, "Should be no problem turning on");
    let false_new = emitter.on_sync(AB(false), |wait_time| {
        thread::sleep(Duration::from_millis(wait_time * 100));
        println!("False");
    });
    assert_ok_equal!(false_new, true, "Should be no problem turning on");
    let true_new = emitter2.on_sync(AB(true), |wait_time| {
        thread::sleep(Duration::from_millis(wait_time * 100));
        println!("True from 2");
    });
    assert_ok_equal!(true_new, true, "Should be no problem turning on");
    let true_new = emitter2.on_sync(AB(true), |wait_time| {
        thread::sleep(Duration::from_millis(wait_time * 100));
        println!("True from 2");
    });
    assert_ok_equal!(true_new, false, "Should not be able to turn on again");

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
    assert_ok_equal!(
        true_used_to_exist,
        true,
        "Should be able to turn true off because they should already be running"
    );
    let true_used_to_exist = emitter.off(AB(true));
    assert_ok_equal!(
        true_used_to_exist,
        false,
        "Should not be able to turn true off because it doesn't exist anymore"
    );
    let false_used_to_exist = emitter.off(AB(false));
    assert!(
        false_used_to_exist.is_err(),
        "Should not be able to turn true off because they are in backlog"
    );

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
            "Emission {:?} with payload emit({:?},{:?}) with result {:?} : received on the channel",
            v.0, v.1, v.2, v.3
        );
    }
    let final_wait = Duration::new(5, 0);
    println!(
        "Waiting for at most {:?} to make sure nothing comes down the channel after wait for all",
        final_wait
    );

    let next_item = rx.recv_timeout(final_wait);
    assert_eq!(next_item, Err(mpsc::RecvTimeoutError::Disconnected));
}

fn main() {
    env_logger::init();

    main_part1();

    common_resource_1();
    /*
    let runtime = tokio::runtime::Builder::new_current_thread()
        .worker_threads(4)
        .thread_name("common-resource-main")
        .enable_all()
        .build()
        .unwrap();
    let _guard = runtime.enter();
    let _ = runtime.block_on(async move {
        common_resource_2();
    });
    */
}
