use crate::events::ThreadEmitter;
use crate::general_emitter::{GeneralEmitter, WhichEvent};
use crate::interleaving::Interleaves;
use crate::utils::JunkMap;
use std::hash::Hash;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::thread::JoinHandle;

type CompositeReturn<EventType, EventArgType, EventReturnType> =
    (Vec<(EventType, EventArgType)>, EventReturnType);

#[allow(dead_code)]
pub struct Cascader<EventType, EventArgType, EventReturnType, EventArgTypeKeep, MapStore>
where
    EventType: Eq + Hash + Clone + Interleaves<EventArgType>,
    MapStore: JunkMap<
        usize,
        (
            WhichEvent,
            EventType,
            EventArgType,
            JoinHandle<CompositeReturn<EventType, EventArgType, EventReturnType>>,
        ),
    >,
{
    out_sender: Option<Sender<(WhichEvent, EventType, EventArgTypeKeep, EventReturnType)>>,
    emitter: ThreadEmitter<
        EventType,
        EventArgType,
        CompositeReturn<EventType, EventArgType, EventReturnType>,
        EventArgTypeKeep,
        MapStore,
    >,
    #[allow(clippy::type_complexity)]
    receiver: Receiver<(
        WhichEvent,
        EventType,
        EventArgTypeKeep,
        CompositeReturn<EventType, EventArgType, EventReturnType>,
    )>,
}

impl<EventType, EventArgType, EventReturnType, EventArgTypeKeep, MapStore>
    Cascader<EventType, EventArgType, EventReturnType, EventArgTypeKeep, MapStore>
where
    EventType: Eq + Hash + Clone + Interleaves<EventArgType>,
    MapStore: JunkMap<
        usize,
        (
            WhichEvent,
            EventType,
            EventArgType,
            JoinHandle<CompositeReturn<EventType, EventArgType, EventReturnType>>,
        ),
    >,
{
    #[allow(dead_code)]
    pub fn new(
        out_sender: Option<Sender<(WhichEvent, EventType, EventArgTypeKeep, EventReturnType)>>,
        arg_transformer_out: fn(EventArgType) -> EventArgTypeKeep,
    ) -> Self {
        let (sender, receiver) = std::sync::mpsc::channel();
        let emitter = ThreadEmitter::<
            EventType,
            EventArgType,
            CompositeReturn<EventType, EventArgType, EventReturnType>,
            EventArgTypeKeep,
            MapStore,
        >::new(Some(sender), arg_transformer_out);
        Self {
            out_sender,
            emitter,
            receiver,
        }
    }
}

impl<EventType, EventArgType, EventReturnType, EventArgTypeKeep, MapStore>
    GeneralEmitter<
        EventType,
        EventArgType,
        CompositeReturn<EventType, EventArgType, EventReturnType>,
    > for Cascader<EventType, EventArgType, EventReturnType, EventArgTypeKeep, MapStore>
where
    EventType: Eq + Hash + Send + Interleaves<EventArgType> + Clone + 'static,
    EventArgType: Clone + Send + 'static,
    EventReturnType: Send + 'static,
    MapStore: JunkMap<
        usize,
        (
            WhichEvent,
            EventType,
            EventArgType,
            JoinHandle<CompositeReturn<EventType, EventArgType, EventReturnType>>,
        ),
    >,
{
    fn reset_panic_policy(&mut self, panic_policy: crate::general_emitter::PanicPolicy) {
        self.emitter.reset_panic_policy(panic_policy)
    }

    fn all_keys(&self) -> impl Iterator<Item = EventType> + '_ {
        self.emitter.all_keys()
    }

    fn is_empty(&self) -> bool {
        self.emitter.is_empty()
    }

    fn count_running(&self) -> usize {
        self.emitter.count_running()
    }

    fn count_waiting(&self) -> usize {
        self.emitter.count_waiting()
    }

    fn off(&mut self, event: EventType) -> Result<bool, crate::general_emitter::EmitterError> {
        self.emitter.off(event)
    }

    fn wait_for_any(&mut self, d: std::time::Duration) -> (bool, Option<usize>) {
        let (something_finished, count_finished) = self.emitter.wait_for_any(d);
        if let Some(did_count) = count_finished {
            for _ in 0..did_count {
                let received_on_channel = self.receiver.try_recv();
                match received_on_channel {
                    Ok((a, b, c, (d, e))) => {
                        for (x, y) in d {
                            let (_had_consumer, _starting_later, _started_already) =
                                self.emit(x, y);
                        }
                        if let Some(send_out) = &self.out_sender {
                            let sending_status = send_out.send((a, b, c, e));
                            assert!(sending_status.is_ok(), "Couldn't send on the channel");
                        }
                    }
                    Err(TryRecvError::Disconnected) => {
                        break;
                    }
                    Err(TryRecvError::Empty) => {
                        continue;
                    }
                }
            }
        }
        (something_finished, count_finished)
    }

    fn wait_for_all(&mut self, d: std::time::Duration) {
        loop {
            let received_on_channel = self.receiver.try_recv();
            match received_on_channel {
                Ok((a, b, c, (d, e))) => {
                    for (x, y) in d {
                        let (_had_consumer, _starting_later, _started_already) = self.emit(x, y);
                    }
                    if let Some(send_out) = &self.out_sender {
                        let sending_status = send_out.send((a, b, c, e));
                        assert!(sending_status.is_ok(), "Couldn't send on the channel");
                    }
                }
                Err(TryRecvError::Disconnected) => {
                    self.emitter.wait_for_all(d);
                    return;
                }
                Err(TryRecvError::Empty) => {
                    let (something_finished, _) = self.wait_for_any(d);
                    if !something_finished {
                        return;
                    }
                }
            }
        }
    }

    fn emit(&mut self, event: EventType, arg: EventArgType) -> (bool, bool, bool) {
        self.emitter.emit(event, arg)
    }
}
