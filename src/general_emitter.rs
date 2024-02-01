use crate::interleaving::Interleaves;
use std::{hash::Hash, time::Duration};

pub type Consumer<EventArgType, EventReturnType> = fn(EventArgType) -> EventReturnType;

pub type WhichEvent = usize;

pub type EmitterError = String;

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
    fn reset_panic_policy(&mut self, panic_policy: PanicPolicy);

    fn all_keys(&self) -> impl Iterator<Item = EventType> + '_;

    fn is_empty(&self) -> bool;

    fn count_running(&self) -> usize;

    fn count_waiting(&self) -> usize;

    fn on(
        &mut self,
        event: EventType,
        callback: Consumer<EventArgType, EventReturnType>,
    ) -> Result<bool, EmitterError>;

    fn off(&mut self, event: EventType) -> Result<bool, EmitterError>;

    #[allow(dead_code)]
    fn all_off(&mut self)
    where
        EventType: Clone,
    {
        let all_registered_events: Vec<EventType> = self.all_keys().collect();
        all_registered_events.into_iter().for_each(|event| {
            let _ = self.off(event);
        })
    }

    fn wait_for_all(&mut self, d: Duration);

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
