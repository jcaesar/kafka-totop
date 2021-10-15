# Light Kafka exporter

Quick CLI util and Prometheus exporter for checking your Kafka:
The number of records that should be flowing, are they actually flowing?

## The non-quick way

The proper way of going on about this is to
 * Get the [prometheus/jmx_exporter](https://github.com/prometheus/jmx_exporter)
 * Acquire a `jmx_exporter` configuration file from somewhere
   * the ["official"](https://github.com/prometheus/jmx_exporter/blob/master/example_configs/kafka-2_0_0.yml) one doesn't work that well
 * Pass the `jmx_exporter` as a `-javaagent` via `KAFKA_JMX_OPTS` to `kafka-server-start.sh`
 * Target prometheus to scrape the agent
 * Hope that you can now query badly named metrics like `kafka_…_brokertopicmetrics_messagesinpersec` (Which is not `per sec`. You need to `irate` it.)

Need to know right now? Have fun restarting your brokers.

## The quick way

This repeatedly queries the high watermarks of all topics via Kafka's normal listening ports and calculates a flow rate.
Clone and run as
```
cargo run -- -b localhost:9092
```



