use crate::interleaving::Interleaves;
use std::{hash::Hash, time::Duration};

pub type Consumer<EventArgType, EventReturnType> = fn(EventArgType) -> EventReturnType;

pub type WhichEvent = usize;

pub type EmitterError = String;

/// what should we do when a panic is encountered
/// we can propograte it up immediately
/// store it and proceed as normally as if that task had returned normally
/// store it and anything that depended on that task executing correctly first
///     will also fail so treat them as having panicked too
#[allow(dead_code)]
#[derive(PartialEq, Eq, Debug, Clone)]
pub enum PanicPolicy {
    PanicAgain,
    StoreButNoSubsequentProblem,
    StoreButSubsequentProblem,
}

pub trait GeneralEmitter<EventType, EventArgType, EventReturnType>
where
    EventType: Eq + Hash + Interleaves<EventArgType> + Clone,
{

    /// change how we react to panics
    fn reset_panic_policy(&mut self, panic_policy: PanicPolicy);

    /// all the EventType's that have a Consumer to run when we call emit with them
    fn all_keys(&self) -> impl Iterator<Item = EventType> + '_;

    /// there are no events running or waiting to be run
    fn is_empty(&self) -> bool;

    /// how many are in the process of execution
    fn count_running(&self) -> usize;

    /// how many are not being run right now because they have to wait for something that is
    /// before them that is either running but not finished or is also waiting
    fn count_waiting(&self) -> usize;

    /// set what to run when we emit a particular EventType
    /// there can't be anything waiting of this particular event
    /// because the Consumer that was associated with it at the time
    /// is what we presumably intended to run, rather than this new Consumer
    /// if none of the events in process use this event, then we can reset it
    fn on(
        &mut self,
        event: EventType,
        callback: Consumer<EventArgType, EventReturnType>,
    ) -> Result<bool, EmitterError>;

    /// we can completely remove the ability to run a Consumer upon this event
    /// as with on, there can't be anything waiting of this particular event
    /// they expected the Consumer that was associated with it at the time of emission
    /// removing that association now would destroy that capability
    fn off(&mut self, event: EventType) -> Result<bool, EmitterError>;

    /// turn as many as possible off
    /// if there were things in the backlog that gave EmitterErrors when trying to turn
    ///     any of them off, then they remain
    /// return if all of them were successfully turned off
    #[allow(dead_code)]
    fn all_off(&mut self) -> bool
    where
        EventType: Clone,
    {
        let mut all_off = true;
        let all_registered_events: Vec<EventType> = self.all_keys().collect();
        all_registered_events.into_iter().for_each(|event| {
            let this_turned_off = self.off(event);
            if this_turned_off.is_err() {
                all_off = false;
            }
        });
        all_off
    }

    /// wait until everything running now and
    /// all the stuff in the backlog gets finished
    /// when nothing has changed, wait for d time
    /// in order to give the running a bit more time to finish
    fn wait_for_all(&mut self, d: Duration);

    /// either the start the associated Consumer running
    /// - or put it in the backlog
    /// - or if it is sure to panic according to PanicPolicy, don't store that
    ///     it would do so
    /// returns
    ///     whether there was an associated Consumer for this event
    ///     will the execution of that Consumer get spawned later
    ///     has the execution of that Consumer spawned already, without any need for waiting
    /// last two are mutually exclusive
    /// if there is no Consumer, both of the others are false but also meaningless
    fn emit(&mut self, event: EventType, arg: EventArgType) -> (bool, bool, bool);
}

#[macro_export]
macro_rules! assert_ok_equal {
    ( $x:expr , $y:expr, $msg:expr) => {
        match $x {
            #[allow(unused_variables)]
            std::result::Result::Err(err_msg) => {
                panic!("Error {} when said {}", stringify!(err_msg), $msg)
            }
            std::result::Result::Ok(e) => {
                assert_eq!(e, $y, $msg)
            }
        }
    };
}
