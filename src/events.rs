use std::collections::HashMap;
use std::hash::Hash;
use std::sync::mpsc::Sender;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use log::{info, warn};

pub trait Interleaves<Aux> {
    fn do_interleave(&self, my_aux: &Aux, other: &Self, others_aux: &Aux) -> bool;
    fn interleaves_with_everything(&self, my_aux: &Aux) -> bool;
}

type Consumer<EventArgType, EventReturnType> = fn(EventArgType) -> EventReturnType;

type WhichEvent = usize;

pub struct Emitter<EventType, EventArgType, EventReturnType>
where
    EventType: Eq + Hash,
{
    my_event_responses: HashMap<EventType, Consumer<EventArgType, EventReturnType>>,
    my_fired_off: Vec<(
        WhichEvent,
        EventType,
        EventArgType,
        JoinHandle<EventReturnType>,
    )>,
    backlog: Vec<(WhichEvent, EventType, EventArgType)>,
    num_emitted_before: WhichEvent,
    results_out: Option<Sender<(WhichEvent, EventType, EventArgType, EventReturnType)>>,
}

impl<EventType, EventArgType, EventReturnType> Drop
    for Emitter<EventType, EventArgType, EventReturnType>
where
    EventType: Eq + Hash,
{
    fn drop(&mut self) {
        let num_backlog = self.backlog.len();
        if num_backlog > 0 {
            warn!("Dropping {} events. They were waiting on something earlier to finish, so never got to start.", num_backlog);
        }
    }
}

impl<EventType, EventArgType, EventReturnType> Emitter<EventType, EventArgType, EventReturnType>
where
    EventType: Eq + Hash + Interleaves<EventArgType>,
{
    pub fn new(
        results_out: Option<Sender<(WhichEvent, EventType, EventArgType, EventReturnType)>>,
    ) -> Self {
        let my_event_responses: HashMap<EventType, Consumer<EventArgType, EventReturnType>> =
            HashMap::new();
        Self {
            my_event_responses,
            my_fired_off: Vec::new(),
            backlog: Vec::new(),
            num_emitted_before: 0,
            results_out,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.my_fired_off.is_empty() && self.backlog.is_empty()
    }
}

impl<EventType, EventArgType, EventReturnType> Emitter<EventType, EventArgType, EventReturnType>
where
    EventType: Eq + Hash + Interleaves<EventArgType>,
    EventArgType: Clone + Send + 'static,
    EventReturnType: Send + 'static,
{
    fn clear_backlog(&mut self) -> bool {
        let mut ready_to_go = Vec::with_capacity(self.backlog.len() >> 2);
        for (cur_idx, cur_backlog) in self.backlog.iter().enumerate() {
            let (cur_backlog_pos, cur_back_log_event, cur_back_log_arg) = cur_backlog;
            let mut still_running_and_dependent =
                self.my_fired_off
                    .iter()
                    .filter(|(full_pos, event_type, arg_type, _)| {
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
            let running_next = still_running_and_dependent.next();
            let backlog_next = backlog_and_dependent.next();
            let none_dependent = running_next.is_none() && backlog_next.is_none();
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

    fn clear_finished(&mut self) -> bool {
        let mut to_remove = Vec::with_capacity(self.my_fired_off.len() >> 3);
        for (idx, (_, _, _, handle)) in self.my_fired_off.iter().enumerate() {
            if handle.is_finished() {
                to_remove.push(idx);
            }
        }
        let something_finished = !to_remove.is_empty();
        to_remove.sort();
        to_remove.reverse();
        for idx in to_remove {
            let (absolute_pos, event_in, event_arg, done_handle) = self.my_fired_off.remove(idx);
            let join_res = done_handle.join();
            if let Ok(real_ret_val) = join_res {
                if let Some(ch) = &self.results_out {
                    let sending_status = ch.send((absolute_pos, event_in, event_arg, real_ret_val));
                    assert!(sending_status.is_ok(), "Couldn't send on the channel");
                }
            } else if let Err(_msg) = join_res {
                panic!("Task {:?} panicked", absolute_pos);
            }
        }
        something_finished
    }

    pub fn emit(&mut self, event: EventType, arg: EventArgType) -> (bool, bool, bool) {
        let can_do_now = if event.interleaves_with_everything(&arg) {
            true
        } else {
            !self.any_earlier_dependences(&event, &arg)
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

    fn any_earlier_dependences(&mut self, event: &EventType, arg: &EventArgType) -> bool {
        self.clear_finished();
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
            .iter()
            .enumerate()
            .filter_map(|(pos_in_spawned, (full_pos, event_type, arg_type, _))| {
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
        absolute_pos: WhichEvent,
        event: EventType,
        arg: EventArgType,
    ) -> (bool, bool) {
        let looked_up = self.my_event_responses.get(&event);
        let arg_clone = arg.clone();
        let my_handle = looked_up.map(|to_do| {
            let to_do_fun = *to_do;
            thread::spawn(move || to_do_fun(arg_clone))
        });
        if let Some(real_handle) = my_handle {
            self.my_fired_off
                .push((absolute_pos, event, arg, real_handle));
            (true, true)
        } else {
            (false, false)
        }
    }

    pub fn on(
        &mut self,
        event: EventType,
        callback: Consumer<EventArgType, EventReturnType>,
    ) -> bool {
        // return value is if this is a new addition
        if self.backlog_uses_this(&event) {
            panic!("Already had a callback for that type of event and had some backlogs for it. Can't change it.");
        } else {
            /*
            no backlog for it
            any emissions for this type of event that might have been there have already been spawned with the function
            they were supposed to at the time of emission
            */
            let old_callback = self.my_event_responses.insert(event, callback);
            old_callback.is_none()
        }
    }

    fn all_keys(&self) -> impl Iterator<Item = EventType> + '_
    where
        EventType: Clone,
    {
        self.my_event_responses.keys().cloned()
    }

    #[allow(dead_code)]
    pub fn all_off(&mut self)
    where
        EventType: Clone,
    {
        let all_registered_events: Vec<EventType> = self.all_keys().collect();
        all_registered_events.into_iter().for_each(|event| {
            let _ = self.off(event);
        })
    }

    pub fn off(&mut self, event: EventType) -> bool {
        // return value is if this actually deleted something
        if self.backlog_uses_this(&event) {
            panic!("Already had a callback for that type of event and had some backlogs for it. Can't remove it.");
        }
        self.my_event_responses.remove(&event).is_some()
    }

    fn backlog_uses_this(&self, event: &EventType) -> bool {
        self.backlog.iter().any(|z| z.1 == *event)
    }

    pub fn wait_for_all(&mut self, d: Duration) {
        /*
        everything running in threads now and all the stuff in the backlog gets finished
        when nothing has changed wait for d time
        in order to give the running threads a bit more time to finish
        */
        loop {
            let something_finished = self.clear_finished();
            if self.is_empty() {
                break;
            }
            if something_finished {
                let something_out_of_backlog = self.clear_backlog();
                let not_keyword = if something_out_of_backlog { "" } else { "not " };
                info!("Something finished, but not everything. An item did {}come out of the backlog. Going through the wait again", not_keyword);
            } else {
                thread::sleep(d);
                info!("Nothing finished, Waited for a bit. Going through the wait again");
            }
        }
    }
}

mod test {
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
        use std::{sync::mpsc, thread, time::Duration};

        use super::Emitter;

        let (tx, rx) = mpsc::channel();
        let mut emitter: Emitter<ABTemp, u64, ()> = Emitter::new(Some(tx));
        let true_new = emitter.on(ABTemp(true), |wait_time| {
            thread::sleep(Duration::from_secs(wait_time));
        });
        assert!(true_new);
        let false_new = emitter.on(ABTemp(false), |wait_time| {
            thread::sleep(Duration::from_secs(wait_time));
        });
        assert!(false_new);
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
        assert!(true_used_to_exist);

        let (exists, spawn_later, spawned) = emitter.emit(ABTemp(true), 10);
        assert!(!exists && !spawned && !spawn_later);

        emitter.wait_for_all(Duration::from_millis(250));
        let expected_order = vec![1, 0, 2];

        for (received_order, v) in rx.iter().enumerate().take(total_emissions) {
            let which_item = expected_order[received_order];
            let expected = &expected_emissions[which_item];
            assert_eq!(v.0, which_item);
            assert_eq!(v.1, expected.0);
            assert_eq!(v.2, expected.1);
            assert_eq!(v.3, expected.2);
        }
        let final_wait = Duration::from_millis(250);
        let next_item = rx.recv_timeout(final_wait);
        assert!(next_item.is_err());
    }

    #[test]
    fn common_resource() {
        use super::Emitter;
        use std::{
            cmp::max,
            sync::{Arc, Mutex},
            thread,
            time::{Duration, Instant},
        };

        let a = |(other_operand, wait_millis, data1): (i32, u64, Arc<Mutex<i32>>)| {
            thread::sleep(Duration::from_millis(wait_millis));
            let mut data = data1.lock().unwrap();
            *data += other_operand;
        };

        let b = |(other_operand, wait_millis, data2): (i32, u64, Arc<Mutex<i32>>)| {
            thread::sleep(Duration::from_millis(wait_millis));
            let mut data = data2.lock().unwrap();
            *data *= other_operand;
        };

        let mut emitter: Emitter<ABTemp, (i32, u64, Arc<Mutex<i32>>), ()> = Emitter::new(None);
        let true_new = emitter.on(ABTemp(true), a);
        assert!(true_new);
        let false_new = emitter.on(ABTemp(false), b);
        assert!(false_new);

        let mut exp_data_val = 1;
        let data = Arc::new(Mutex::new(exp_data_val));

        let mut other_operand;
        let mut expected_time = Duration::from_millis(0);
        let mut serial_time = Duration::from_millis(0);

        other_operand = 1;
        let wait_time_1 = 50;
        emitter.emit(ABTemp(true), (other_operand, wait_time_1, data.clone()));
        exp_data_val += other_operand;

        other_operand = 10;
        let wait_time_2 = 60;
        emitter.emit(ABTemp(true), (other_operand, wait_time_2, data.clone()));
        exp_data_val += other_operand;

        expected_time += Duration::from_millis(max(wait_time_1, wait_time_2));
        serial_time += Duration::from_millis(wait_time_1);
        serial_time += Duration::from_millis(wait_time_2);

        other_operand = 2;
        let wait_time_3 = 50;
        emitter.emit(ABTemp(false), (other_operand, wait_time_3, data.clone()));
        exp_data_val *= other_operand;
        expected_time += Duration::from_millis(wait_time_3);
        serial_time += Duration::from_millis(wait_time_3);

        other_operand = 24;
        let wait_time_4 = 20;
        emitter.emit(ABTemp(true), (other_operand, wait_time_4, data.clone()));
        exp_data_val += other_operand;
        expected_time += Duration::from_millis(wait_time_4);
        serial_time += Duration::from_millis(wait_time_4);

        let loop_time = Duration::from_millis(15);
        let now = Instant::now();
        emitter.wait_for_all(loop_time);
        let elapsed = now.elapsed();
        assert!(elapsed < expected_time + 3 * loop_time);
        assert!(elapsed < serial_time);
        let data_val = *data.lock().unwrap();
        assert_eq!(data_val, exp_data_val);
    }
}
