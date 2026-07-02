# EventEmit

An event emitter for Rust where consumers run concurrently by default, but are
automatically serialized whenever they *don't commute*. You tell the emitter
which pairs of events are safe to interleave; it figures out the rest —
spawning independent work immediately and holding dependent work in a backlog
until whatever it depends on has finished.

## Motivating example

```rust
d.emit(x1, y1);
d.emit(x2, y2);
d.emit(x3, y3);
d.emit(x4, y4);
d.emit(x5, y5);
d.wait_for_all(sleep_time);
```

Each emission `xi` triggers some (likely side-effecting) computation run with
argument `yi`. If every computation were atomic and always succeeded, the only
question would be whether they can run in an arbitrary order. If they all
commuted with each other, we could just spawn a thread for each of the five
right away.

Instead of assuming everything commutes and can be freely interleaved, this
crate lets you be more precise. Imagine each computation does I/O against
locations you can determine just by looking at `xi, yi` — the emitter will
only spawn a computation once everything it depends on has already finished.

For example, suppose:
- the first two commute and can be interleaved arbitrarily
- the third does not commute with the second, but does commute with the first
- the fourth commutes with everything
- the fifth commutes with everything except the second — e.g. both do
  `old = *data; *data = old + f(args)`, which commutes mathematically, but
  running them concurrently on separate threads wouldn't reproduce the same
  result as either serial order

Then:
- the first two spawn as soon as they're emitted
- the third goes into a backlog, waiting on the second to finish
- the fourth spawns as soon as it's emitted
- the fifth also goes into the backlog
- once the second finishes, the third and fifth are pulled out of the backlog
  and spawned — it doesn't matter whether the first or fourth have finished,
  since neither of them conflicts

You can also give the emitter a channel; the output of each computation is
sent on it as soon as that computation is known to be finished.

## Core concepts

- **`EventType`** — the key you `emit` with. It identifies which registered
  `Consumer` should run.
- **`Consumer<EventArgType, EventReturnType>`** — the function registered for
  an `EventType` via `on_sync`/`on_async`, run with the argument passed to
  `emit`.
- **[`Interleaves`](src/interleaving.rs)** — the trait you implement on your
  `EventType` to declare whether two emissions commute:
  - `do_interleave(&self, my_aux, other, others_aux) -> bool` — could these two
    specific emissions run concurrently without changing the observable
    result?
  - `interleaves_with_everything(&self, my_aux) -> bool` — a shortcut for
    emissions (e.g. pure reads) that never conflict with anything.
- **[`GeneralEmitter` / `SyncEmitter` / `AsyncEmitter`](src/general_emitter.rs)**
  — the emitter API: `on_sync`/`on_async` to register a `Consumer`, `emit` to
  fire an event (spawning it immediately or queuing it in a backlog),
  `wait_for_any`/`wait_for_all` to block until work completes, `off`/`all_off`
  to deregister, and `reset_panic_policy` to control what happens when a
  `Consumer` panics (propagate immediately, or record it and either treat
  dependents as tainted or let them proceed normally).

## Usage

The package builds both a library (`src/lib.rs`) and a demo binary
(`src/main.rs`); the binary just depends on the library like any other
consumer would:

```rust
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use event_emitter::events::SpecificThreadEmitter;
use event_emitter::general_emitter::{GeneralEmitter, SyncEmitter};
use event_emitter::interleaving::Interleaves;

#[derive(PartialEq, Eq, Hash, Clone, Debug)]
struct Bucket(bool);

impl<T> Interleaves<T> for Bucket {
    // events on the same bucket must be serialized; different buckets don't conflict
    fn do_interleave(&self, _: &T, other: &Self, _: &T) -> bool {
        self != other
    }
    fn interleaves_with_everything(&self, _my_aux: &T) -> bool {
        false
    }
}

let (tx, rx) = mpsc::channel();
let identity = |x| x;
let mut emitter: SpecificThreadEmitter<Bucket, u64, (), _> =
    SpecificThreadEmitter::new(Some(tx), identity);

emitter.on_sync(Bucket(true), |ms| thread::sleep(Duration::from_millis(ms))).unwrap();
emitter.on_sync(Bucket(false), |ms| thread::sleep(Duration::from_millis(ms))).unwrap();

emitter.emit(Bucket(true), 50);   // spawns immediately
emitter.emit(Bucket(false), 10);  // different bucket, commutes, spawns immediately too
emitter.emit(Bucket(true), 5);    // same bucket as the first, conflicts, goes to the backlog

emitter.wait_for_all(Duration::from_millis(5));
for (idx, event, arg, ()) in rx.try_iter() {
    println!("event #{idx} {event:?}({arg}) finished");
}
```

`ThreadEmitter` runs consumers with `std::thread::spawn`; `TokioEmitter` (see
below) is the `tokio::spawn`-based counterpart.

## Crate layout

| Module | Purpose |
| --- | --- |
| [`general_emitter`](src/general_emitter.rs) | Core traits (`GeneralEmitter`, `SyncEmitter`, `AsyncEmitter`), `PanicPolicy`, and shared types like `Consumer`. |
| [`interleaving`](src/interleaving.rs) | The `Interleaves` trait used to decide which emissions can run concurrently. |
| [`events`](src/events.rs) | `ThreadEmitter`, the thread-backed implementation of the emitter traits. |
| [`tokio_events`](src/tokio_events.rs) | `TokioEmitter`, a `tokio::spawn`-backed implementation (work in progress). |
| [`cascading`](src/cascading.rs) | `Cascader`, an emitter wrapper for consumers that themselves emit further events (work in progress). |
| [`utils`](src/utils.rs) | `GeneralMap`, an abstraction over the map used to track in-flight work, implemented for `HashMap`. |

## Status

This is an experimental/exploratory crate. `tokio_events` and `cascading` are
still under active development, and `AsyncConsumer::new` is currently
unimplemented (`todo!()`).

## Running tests

```sh
cargo test
```
