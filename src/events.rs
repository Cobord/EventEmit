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

pub struct Emitter<EventType, EventArgType, EventReturnType>
where
    EventType: Eq + Hash,
{
    my_event_responses: HashMap<EventType, Consumer<EventArgType, EventReturnType>>,
    my_fired_off: Vec<(usize, EventType, EventArgType, JoinHandle<EventReturnType>)>,
    backlog: Vec<(usize, EventType, EventArgType)>,
    num_emitted_before: usize,
    results_out: Option<Sender<(usize, EventType, EventArgType, EventReturnType)>>,
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
        results_out: Option<Sender<(usize, EventType, EventArgType, EventReturnType)>>,
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
            let go_now: (usize, EventType, EventArgType) = self.backlog.remove(idx);
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
                panic!("Task {} panicked", absolute_pos);
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
            info!("{} Into backlog", self.num_emitted_before);
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
        absolute_pos: usize,
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

    #[test]
    fn two_events() {
        use std::{sync::mpsc, thread, time::Duration};

        use crate::events::{Emitter, Interleaves};

        #[derive(PartialEq, Eq, Hash, Copy, Clone)]
        #[repr(transparent)]
        struct AB(bool);
        impl Interleaves<u64> for AB {
            fn do_interleave(&self, _: &u64, other: &Self, _: &u64) -> bool {
                self == other
            }

            fn interleaves_with_everything(&self, _my_aux: &u64) -> bool {
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
        let mut emitter: Emitter<AB, u64, ()> = Emitter::new(Some(tx));
        let true_new = emitter.on(AB(true), |wait_time| {
            thread::sleep(Duration::from_secs(wait_time));
        });
        assert!(true_new);
        let false_new = emitter.on(AB(false), |wait_time| {
            thread::sleep(Duration::from_secs(wait_time));
        });
        assert!(false_new);
        let mut total_emissions = 0;
        let mut expected_emissions: Vec<(AB, u64, ())> = Vec::with_capacity(3);

        let emission_now = (AB(true), 2);
        expected_emissions.push((emission_now.0, emission_now.1, ()));

        let (exists, spawn_later, spawned) = emitter.emit(emission_now.0, emission_now.1);
        total_emissions += 1;
        assert!(exists && spawned && !spawn_later);

        let emission_now = (AB(true), 1);
        expected_emissions.push((emission_now.0, emission_now.1, ()));
        let (exists, spawn_later, spawned) = emitter.emit(emission_now.0, emission_now.1);
        total_emissions += 1;
        assert!(exists && spawned && !spawn_later);

        let emission_now: (AB, u64) = (AB(false), 1);
        expected_emissions.push((emission_now.0, emission_now.1, ()));
        let (exists, spawn_later, spawned) = emitter.emit(emission_now.0, emission_now.1);
        total_emissions += 1;
        assert!(exists && !spawned && spawn_later);

        let true_used_to_exist = emitter.off(AB(true));
        assert!(true_used_to_exist);

        let (exists, spawn_later, spawned) = emitter.emit(AB(true), 10);
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
}
