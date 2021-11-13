# ToTop

*Kafka Table of Topics*

A quick TUI for grasping the message rate in your topics.

This is not a Kafka swiss army knife like [kcat](https://github.com/edenhill/kcat), [kaf](https://github.com/birdayz/kaf), [kcli](https://github.com/cswank/kcli) or [zoe](https://github.com/adevinta/zoe). But it does one thing that those don't do:

Display a graph!


Of course, a proper cluster setup would have some kind of monitoring architecture that would give you this information (e.g. based on the [prometheus/jmx_exporter](https://github.com/prometheus/jmx_exporter)).
But if you encounter a clusterfuck, you wish for an easy alternative that doesn't require restarting your brokers, when just the normal kafka listeners are sufficient.

This repo is a spin-off of [light-kafka-exporter](https://github.com/jcaesar/light-kafka-exporter).
