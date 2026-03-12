if you are worried about messages interleaving consider spawning a task with the receiver of a mpsc where the message is the String, then this task will send one whole string at a time, this way having the single responsibility and ensuring no interleaving could eveyr happen.

when this is possible you should always prefer message passing with mpsc or oneshot (tokio compatible variants of course) over mutex
just make sure you never have a situation where you need to await multiple receivers at the same time, as ordering could then introduce a deadlock, in that situation we need to receive on all simmultaneously
