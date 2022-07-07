# ToTop

*Kafka Table of Topics*

A quick TUI for grasping the message ingestion rate in your topics. Run it in your k8s cluster:

```bash
kubectl run totop -i --tty --image=docker.io/liftm/kafka-totop -- -b $your_kafka_host:9092
```

This is not a Kafka swiss army knife like [kcat](https://github.com/edenhill/kcat), [kaf](https://github.com/birdayz/kaf), [kcli](https://github.com/cswank/kcli) or [zoe](https://github.com/adevinta/zoe). But it does one thing that those don't(?) do: display a graph of message throughput!
![screenshot](/screenshot.png)

Of course, a proper cluster setup would have some kind of monitoring architecture that would give you this information (e.g. based on the [prometheus/jmx_exporter](https://github.com/prometheus/jmx_exporter)).
But when you're debugging, chances are you don't have a proper setup yet. ToTop is for those "one-off" situations: it pulls the necessary information from the normal Kafka listening port.

This repo is a spin-off of [light-kafka-exporter](https://github.com/jcaesar/light-kafka-exporter).
