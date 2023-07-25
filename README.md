```
d.emit(x1,y1);
d.emit(x2,y2);
d.emit(x3,y3);
d.emit(x4,y4);
d.emit(x5,y5);
d.wait_for_all(sleep_time);
```

Each of these emissions is some (likely side-effect-full) computation associated with the xi with argument yi.
Suppose they were atomic and would always succeed, then whether or not they could be done in arbitrary order would be the main question.
If further they all commuted, then we could just spawn threads for all 5.

But now instead of implementing the traits with saying everything commutes and can be arbitrarily interleaved, one could be more restrictive.
Imagine each thread is doing some IO and they are interacting with independent input/output locations that you can immediately judge from the xi,yi.

Now it will spawn the threads only when everything that had to occur earlier has already finished.

For example, suppose the following commutation pattern (and there is no extra complication from interleaving)
- the first two commuted
- the third didn't commute with the second, but did with the first
- the fourth commuted with everything
- the fifth commuted with everything except the second

Then
- the first two will spawn as soon as they are emitted
- the third will go into a backlog waiting for two to finish because it must occur completely after it
- the fourth will spawn as soon as it is emitted
- the fifth will also go into the backlog
- when the second finishes, the third and fifth are pulled out of the backlog and have their threads spawned
  - it does not matter if the first and fourth haven't finished because they commute with them

One can also provide a channel in which case the outputs of each go into that channel as soon as they have been established to be finished.
