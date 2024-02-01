use log::{info, warn};
use tokio::runtime::Handle;

use std::{
    any::Any,
    collections::HashMap,
    hash::Hash,
    sync::{mpsc::Sender, Arc, Mutex},
    time::Duration,
};

use crate::general_emitter::{Consumer, EmitterError, GeneralEmitter, PanicPolicy, WhichEvent};
use crate::interleaving::Interleaves;

pub struct TokioEmitter<EventType, EventArgType, EventReturnType, EventArgTypeKeep>
where
    EventType: Eq + Hash,
{
    my_event_responses: HashMap<EventType, Consumer<EventArgType, EventReturnType>>,
    my_fired_off: Arc<Mutex<Vec<(WhichEvent, EventType, EventArgType)>>>,
    backlog: Vec<(WhichEvent, EventType, EventArgType)>,
    num_emitted_before: WhichEvent,
    arg_transformer_out: fn(EventArgType) -> EventArgTypeKeep,
    results_out: Option<Sender<(WhichEvent, EventType, EventArgTypeKeep, EventReturnType)>>,
    panic_policy: PanicPolicy,
    panicked_events: Vec<(WhichEvent, EventType, EventArgType, Box<dyn Any + Send>)>,
}

impl<EventType, EventArgType, EventReturnType, EventArgTypeKeep> Drop
    for TokioEmitter<EventType, EventArgType, EventReturnType, EventArgTypeKeep>
where
    EventType: Eq + Hash,
{
    fn drop(&mut self) {
        let num_backlog = self.backlog.len();
        if num_backlog > 0 {
            warn!("Dropping {} events. They were waiting on something earlier to finish, so never got to start.", num_backlog);
        }
        let num_running = self.my_fired_off.lock().unwrap().len();
        if num_running > 0 {
            warn!("Dropping {} events. They were running but unfinished so their outputs never got to the output channel.", num_running);
        }
    }
}

impl<EventType, EventArgType, EventReturnType, EventArgTypeKeep>
    TokioEmitter<EventType, EventArgType, EventReturnType, EventArgTypeKeep>
where
    EventType: Eq + Hash,
{
    #[allow(dead_code)]
    pub fn new(
        results_out: Option<Sender<(WhichEvent, EventType, EventArgTypeKeep, EventReturnType)>>,
        arg_transformer_out: fn(EventArgType) -> EventArgTypeKeep,
    ) -> Self {
        let my_event_responses: HashMap<EventType, Consumer<EventArgType, EventReturnType>> =
            HashMap::new();
        Self {
            my_event_responses,
            my_fired_off: Arc::new(Mutex::new(Vec::new())),
            backlog: Vec::new(),
            num_emitted_before: 0,
            arg_transformer_out,
            results_out,
            panic_policy: PanicPolicy::PanicAgain,
            panicked_events: Vec::new(),
        }
    }
}

impl<EventType, EventArgType, EventReturnType, EventArgTypeKeep>
    GeneralEmitter<EventType, EventArgType, EventReturnType>
    for TokioEmitter<EventType, EventArgType, EventReturnType, EventArgTypeKeep>
where
    EventType: Eq + Hash + Interleaves<EventArgType> + Clone + Send + 'static,
    EventArgType: Clone + Send + 'static,
    EventReturnType: Send + 'static,
    EventArgTypeKeep: Send + 'static,
{
    fn reset_panic_policy(&mut self, panic_policy: PanicPolicy) {
        self.panic_policy = panic_policy;
    }

    fn all_keys(&self) -> impl Iterator<Item = EventType> + '_ {
        self.my_event_responses.keys().cloned()
    }

    fn is_empty(&self) -> bool {
        self.backlog.is_empty() && self.my_fired_off.lock().unwrap().is_empty()
    }

    fn count_running(&self) -> usize {
        self.my_fired_off.lock().unwrap().len()
    }

    fn count_waiting(&self) -> usize {
        self.backlog.len()
    }

    fn on(
        &mut self,
        event: EventType,
        callback: Consumer<EventArgType, EventReturnType>,
    ) -> Result<bool, EmitterError> {
        // return value is if this is a new addition vs an overwrite of something already there
        if self.backlog_uses_this(&event) {
            Err("Already had a callback for that type of event and had some backlogs for it. Can't change it.".to_string())
        } else {
            /*
            no backlog for it
            any emissions for this type of event that might have been there have already been spawned with the function
            they were supposed to at the time of emission
            */
            let old_callback = self.my_event_responses.insert(event, callback);
            Ok(old_callback.is_none())
        }
    }

    fn off(&mut self, event: EventType) -> Result<bool, EmitterError> {
        // return value is if this actually deleted something
        if self.backlog_uses_this(&event) {
            Err("Already had a callback for that type of event and had some backlogs for it. Can't remove it.".to_string())
        } else {
            Ok(self.my_event_responses.remove(&event).is_some())
        }
    }

    fn wait_for_all(&mut self, d: Duration) {
        /*
        everything running in threads now and all the stuff in the backlog gets finished
        when nothing has changed wait for d time
        in order to give the running threads a bit more time to finish
        */
        let mut finished_nums: Vec<usize> = self
            .my_fired_off
            .lock()
            .unwrap()
            .iter()
            .map(|(a, _, _)| *a)
            .collect();
        loop {
            println!("Going Around");
            let new_finished_nums = self
                .my_fired_off
                .lock()
                .unwrap()
                .iter()
                .map(|(a, _, _)| *a)
                .collect();
            let something_finished = new_finished_nums != finished_nums;
            finished_nums = new_finished_nums;
            if self.is_empty() {
                break;
            }
            if something_finished {
                println!("Something finished");
                let something_out_of_backlog = self.clear_backlog();
                let not_keyword = if something_out_of_backlog { "" } else { "not " };
                info!("Something finished, but not everything. An item did {}come out of the backlog. Going through the wait again", not_keyword);
            } else if finished_nums.is_empty() {
                println!("Finished was empty");
                let something_out_of_backlog = self.clear_backlog();
                let not_keyword = if something_out_of_backlog { "" } else { "not " };
                info!("Nothing finished. An item did {}come out of the backlog. Going through the wait again", not_keyword);
            } else {
                println!("Neither just to sleep");
                let current_handle = Handle::current();
                current_handle.block_on(tokio::time::sleep(d));
                info!("Nothing finished, Waited for a bit. Going through the wait again");
            }
        }
    }

    fn emit(&mut self, event: EventType, arg: EventArgType) -> (bool, bool, bool) {
        let can_do_now = if event.interleaves_with_everything(&arg) {
            true
        } else {
            !self.any_earlier_normal_dependences(&event, &arg)
        };

        let (exists, will_spawn_later, spawned_already) = if can_do_now {
            let (exists, spawned) = self.unchecked_emit(self.num_emitted_before, event, arg);
            (exists, false, spawned)
        } else if self.my_event_responses.contains_key(&event) {
            info!("{:?} Into backlog", self.num_emitted_before);
            self.backlog.push((self.num_emitted_before, event, arg));
            (true, true, false)
        } else {
            (false, false, false)
        };
        self.num_emitted_before += 1;
        (exists, will_spawn_later, spawned_already)
    }
}

impl<EventType, EventArgType, EventReturnType, EventArgTypeKeep>
    TokioEmitter<EventType, EventArgType, EventReturnType, EventArgTypeKeep>
where
    EventType: Eq + Hash + Clone + Send + Interleaves<EventArgType> + 'static,
    EventArgType: Clone + Send + 'static,
    EventReturnType: Send + 'static,
    EventArgTypeKeep: Send + 'static,
{
    fn backlog_uses_this(&self, event: &EventType) -> bool {
        self.backlog.iter().any(|z| z.1 == *event)
    }

    fn clear_backlog(&mut self) -> bool {
        let mut ready_to_go = Vec::with_capacity(self.backlog.len() >> 2);
        for (cur_idx, cur_backlog) in self.backlog.iter().enumerate() {
            let (cur_backlog_pos, cur_back_log_event, cur_back_log_arg) = cur_backlog;
            let running_next_any_bad =
                self.my_fired_off
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|(full_pos, event_type, arg_type)| {
                        *full_pos < *cur_backlog_pos
                            && !cur_back_log_event.do_interleave(
                                cur_back_log_arg,
                                event_type,
                                arg_type,
                            )
                    });
            let mut backlog_and_dependent =
                self.backlog
                    .iter()
                    .filter(|(full_pos, event_type, arg_type)| {
                        *full_pos < *cur_backlog_pos
                            && !cur_back_log_event.do_interleave(
                                cur_back_log_arg,
                                event_type,
                                arg_type,
                            )
                    });
            let backlog_next = backlog_and_dependent.next();
            let none_dependent = !running_next_any_bad && backlog_next.is_none();
            if none_dependent {
                ready_to_go.push(cur_idx);
            }
        }
        ready_to_go.reverse();
        let something_out_of_backlog = !ready_to_go.is_empty();
        for idx in ready_to_go {
            let go_now: (WhichEvent, EventType, EventArgType) = self.backlog.remove(idx);
            self.unchecked_emit(go_now.0, go_now.1, go_now.2);
        }
        something_out_of_backlog
    }

    fn any_earlier_normal_dependences(&mut self, event: &EventType, arg: &EventArgType) -> bool {
        self.clear_backlog();
        let need_to_wait_for_backlog = self
            .backlog
            .iter()
            .enumerate()
            .filter_map(|(pos_in_spawned, (full_pos, event_type, event_arg))| {
                if !event_type.do_interleave(event_arg, event, arg) {
                    Some((pos_in_spawned, *full_pos))
                } else {
                    None
                }
            })
            .next()
            .is_some();

        if need_to_wait_for_backlog {
            return true;
        }

        let need_to_wait_for_spawned = self
            .my_fired_off
            .lock()
            .unwrap()
            .iter()
            .enumerate()
            .filter_map(|(pos_in_spawned, (full_pos, event_type, arg_type))| {
                if !event_type.do_interleave(arg_type, event, arg) {
                    Some((pos_in_spawned, *full_pos))
                } else {
                    None
                }
            })
            .next()
            .is_some();

        need_to_wait_for_backlog || need_to_wait_for_spawned
    }

    fn unchecked_emit(
        &mut self,
        absolute_pos: usize,
        event: EventType,
        arg: EventArgType,
    ) -> (bool, bool) {
        let looked_up = self.my_event_responses.get(&event);
        if looked_up.is_some() {
            match self.panic_policy {
                PanicPolicy::PanicAgain => {
                    /*
                    panic on the first error when join handle so just emit this as normal
                    */
                }
                PanicPolicy::StoreButNoSubsequentProblem => {
                    /*
                    anything in self.panicked_events isn't going to be a problem anyway
                    */
                }
                PanicPolicy::StoreButSubsequentProblem => {
                    for (event_num, bad_event_type, bad_event_arg, _) in self.panicked_events.iter()
                    {
                        let is_going_to_be_bad = *event_num < absolute_pos
                            && !bad_event_type.do_interleave(bad_event_arg, &event, &arg);
                        if is_going_to_be_bad {
                            let msg =
                                format!("An earlier panic {} is getting in the way", *event_num);
                            self.panicked_events
                                .push((absolute_pos, event, arg, Box::new(msg)));
                            return (true, false);
                        }
                    }
                }
            }
        }

        let my_return = looked_up.map(|to_do| {
            let to_do_fun = *to_do;
            let arg_clone = arg.clone();
            let event_clone = event.clone();
            let running_vec = Arc::clone(&self.my_fired_off);
            self.my_fired_off
                .lock()
                .unwrap()
                .push((absolute_pos, event.clone(), arg.clone()));
            let where_to_send = self.results_out.clone();
            let arg_transformer_out = self.arg_transformer_out;

            tokio::spawn(async move {
                let real_ret_val = to_do_fun(arg_clone.clone());
                let idx_in_running = running_vec
                    .lock()
                    .unwrap()
                    .iter()
                    .enumerate()
                    .find(|(_, (a, b, _))| *a == absolute_pos && *b == event_clone)
                    .map(|(idx, (_, _, _))| idx);
                if let Some(idx_to_remove) = idx_in_running {
                    running_vec.lock().unwrap().remove(idx_to_remove);
                }
                if let Some(ch) = where_to_send {
                    let event_arg_fixed = (arg_transformer_out)(arg_clone);
                    let sending_status =
                        ch.send((absolute_pos, event, event_arg_fixed, real_ret_val));
                    assert!(sending_status.is_ok(), "Couldn't send on the channel");
                }
            });
            (true, true)
        });
        if let Some(real_return) = my_return {
            real_return
        } else {
            (false, false)
        }
    }
}

mod test {
    use crate::general_emitter::PanicPolicy;

    use super::Interleaves;

    #[derive(PartialEq, Eq, Hash, Copy, Clone)]
    #[repr(transparent)]
    struct ABTemp(bool);
    impl<T> Interleaves<T> for ABTemp {
        fn do_interleave(&self, _: &T, other: &Self, _: &T) -> bool {
            self == other
        }

        fn interleaves_with_everything(&self, _my_aux: &T) -> bool {
            false
        }
    }
    use std::fmt::{Debug, Error, Formatter};
    impl Debug for ABTemp {
        fn fmt(&self, f: &mut Formatter) -> Result<(), Error> {
            write!(f, "{:?}", self.0)
        }
    }

    #[test]
    fn two_events() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .worker_threads(4)
            .thread_name("two-events-test")
            .enable_all()
            .build()
            .unwrap();
        let _guard = runtime.enter();
        let _ = runtime.block_on(tokio::spawn(async move {
            two_events_helper();
        }));
    }

    #[allow(dead_code)]
    fn two_events_helper() {
        use std::{sync::mpsc, thread, time::Duration};

        use super::{GeneralEmitter, TokioEmitter};
        use crate::assert_ok_equal;

        let (tx, rx) = mpsc::channel();
        let identity = |x| x;
        let mut emitter: TokioEmitter<ABTemp, u64, (), _> = TokioEmitter::new(Some(tx), identity);
        let true_new = emitter.on(ABTemp(true), |wait_time| {
            thread::sleep(Duration::from_millis(wait_time * 100));
        });
        assert_ok_equal!(true_new, true, "Should be no problem turning on");
        let false_new = emitter.on(ABTemp(false), |wait_time| {
            thread::sleep(Duration::from_secs(wait_time));
        });
        assert_ok_equal!(false_new, true, "Should be no problem turning on");
        let mut total_emissions = 0;
        let mut expected_emissions: Vec<(ABTemp, u64, ())> = Vec::with_capacity(3);

        let emission_now = (ABTemp(true), 2);
        expected_emissions.push((emission_now.0, emission_now.1, ()));

        let (exists, spawn_later, spawned) = emitter.emit(emission_now.0, emission_now.1);
        total_emissions += 1;
        assert!(exists && spawned && !spawn_later);

        let emission_now = (ABTemp(true), 1);
        expected_emissions.push((emission_now.0, emission_now.1, ()));
        let (exists, spawn_later, spawned) = emitter.emit(emission_now.0, emission_now.1);
        total_emissions += 1;
        assert!(exists && spawned && !spawn_later);

        let emission_now: (ABTemp, u64) = (ABTemp(false), 1);
        expected_emissions.push((emission_now.0, emission_now.1, ()));
        let (exists, spawn_later, spawned) = emitter.emit(emission_now.0, emission_now.1);
        total_emissions += 1;
        assert!(exists && !spawned && spawn_later);

        let true_used_to_exist = emitter.off(ABTemp(true));
        assert_ok_equal!(
            true_used_to_exist,
            true,
            "Should be able to turn true off because they should already be running"
        );
        let true_used_to_exist = emitter.off(ABTemp(true));
        assert_ok_equal!(
            true_used_to_exist,
            false,
            "Should not be able to turn true off because it doesn't exist anymore"
        );
        let false_used_to_exist = emitter.off(ABTemp(false));
        assert!(
            false_used_to_exist.is_err(),
            "Should not be able to turn true off because they are in backlog"
        );

        let (exists, spawn_later, spawned) = emitter.emit(ABTemp(true), 10);
        assert!(!exists && !spawned && !spawn_later);

        emitter.wait_for_all(Duration::from_millis(250));
        let expected_order = [1, 0, 2];

        for (received_order, v) in rx.iter().enumerate().take(total_emissions) {
            let which_item = expected_order[received_order];
            let expected = &expected_emissions[which_item];
            assert_eq!(v.0, which_item);
            assert_eq!(v.1, expected.0);
            assert_eq!(v.2, expected.1);
        }
        let final_wait = Duration::from_millis(250);
        let next_item = rx.recv_timeout(final_wait);
        assert_eq!(next_item, Err(mpsc::RecvTimeoutError::Timeout));

        assert!(emitter.panicked_events.is_empty());
        assert_eq!(emitter.panic_policy, PanicPolicy::PanicAgain);
    }

    #[test]
    fn common_resource() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .worker_threads(4)
            .thread_name("common-resource-test")
            .enable_all()
            .build()
            .unwrap();
        let _guard = runtime.enter();
        tokio::spawn(async {
            common_resource_helper(PanicPolicy::PanicAgain);
        });
        tokio::spawn(async {
            common_resource_helper(PanicPolicy::StoreButNoSubsequentProblem);
        });
        tokio::spawn(async {
            common_resource_helper(PanicPolicy::StoreButSubsequentProblem);
        });
    }

    #[allow(dead_code)]
    fn common_resource_helper(policy: PanicPolicy) {
        use super::{GeneralEmitter, TokioEmitter};
        use crate::assert_ok_equal;
        use std::{
            cmp::max,
            sync::{Arc, Mutex},
            thread,
            time::{Duration, Instant},
        };

        let a = |(other_operand, wait_millis, data1): (i32, u64, Arc<Mutex<i32>>)| {
            thread::sleep(Duration::from_millis(wait_millis));
            let mut data = data1.lock().expect("Acquiring lock here shouldn't panic");
            *data += other_operand;
        };

        let b = |(other_operand, wait_millis, data2): (i32, u64, Arc<Mutex<i32>>)| {
            thread::sleep(Duration::from_millis(wait_millis));
            let mut data = data2.lock().expect("Acquiring lock here shouldn't panic");
            *data *= other_operand;
        };

        type MyArgType = (i32, u64, Arc<Mutex<i32>>);
        let dont_show_common_resource = |(_0, _1, _2)| (_0, _1);
        let (tx, rx) = std::sync::mpsc::channel();
        let mut emitter: TokioEmitter<ABTemp, MyArgType, (), (i32, u64)> =
            TokioEmitter::new(Some(tx), dont_show_common_resource);
        emitter.reset_panic_policy(policy.clone());
        let true_new = emitter.on(ABTemp(true), a);
        assert_ok_equal!(true_new, true, "Should be no problem turning on");
        let false_new = emitter.on(ABTemp(false), b);
        assert_ok_equal!(false_new, true, "Should be no problem turning on");

        let mut exp_data_val = 1;
        let data = Arc::new(Mutex::new(exp_data_val));

        let mut other_operand;
        let mut expected_time = Duration::from_millis(0);
        let mut serial_time = Duration::from_millis(0);
        let mut total_operations = 0;

        other_operand = 1;
        let wait_time_1 = 50;
        emitter.emit(ABTemp(true), (other_operand, wait_time_1, data.clone()));
        exp_data_val += other_operand;
        total_operations += 1;
        assert_eq!(
            emitter.count_running() + emitter.count_waiting(),
            total_operations
        );

        other_operand = 10;
        let wait_time_2 = 60;
        emitter.emit(ABTemp(true), (other_operand, wait_time_2, data.clone()));
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
        emitter.emit(ABTemp(false), (other_operand, wait_time_3, data.clone()));
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
        emitter.emit(ABTemp(true), (other_operand, wait_time_4, data.clone()));
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
        emitter.wait_for_all(loop_time);
        let elapsed = now.elapsed();
        assert!(elapsed < expected_time + 3 * loop_time);
        assert!(elapsed < serial_time);
        let data_val = *data.lock().expect("Acquiring lock here shouldn't panic");
        assert_eq!(data_val, exp_data_val);
        assert!(emitter.panicked_events.is_empty());
        assert_eq!(emitter.panic_policy, policy);

        for v in rx.iter().take(total_operations) {
            assert!(v.0 < total_operations);
            #[allow(clippy::comparison_chain)]
            if v.0 < 2 {
                assert_eq!(v.1, ABTemp(true));
                assert!([1, 10].contains(&v.2 .0));
                assert!([wait_time_1, wait_time_2].contains(&v.2 .1));
            } else if v.0 == 2 {
                assert_eq!(v.1, ABTemp(false));
                assert_eq!(2, v.2 .0);
                assert_eq!(wait_time_3, v.2 .1);
            } else {
                assert_eq!(v.1, ABTemp(true));
                assert_eq!(24, v.2 .0);
                assert_eq!(wait_time_4, v.2 .1);
            }
        }
        let final_wait = Duration::new(1, 0);
        let next_item = rx.recv_timeout(final_wait);
        assert_eq!(
            next_item,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
        );
    }
}
