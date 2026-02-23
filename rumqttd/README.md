# rumqttd

[![crates.io page](https://img.shields.io/crates/v/rumqttd.svg)](https://crates.io/crates/rumqttd)
[![docs.rs page](https://docs.rs/rumqttd/badge.svg)](https://docs.rs/rumqttd)

Rumqttd is a high performance MQTT broker written in Rust. It's light weight and embeddable, meaning
you can use it as a library in your code and extend functionality

## Getting started

You can directly run the broker by running the binary with a config file with:

```
cargo run --release -- -c rumqttd.toml

```

Example config file is provided on the root of the repo.


#### Building the docker image

In order to run rumqttd within a docker container, build the image by running `build_rumqttd_docker.sh` from the project's root directory. The shell script will use docker to build rumqttd and package it along in an [alpine](https://hub.docker.com/_/alpine) image. You can then run `rumqttd` using default config with:

```bash
./build_rumqttd_docker.sh
docker run -p 1883:1883 -p 1884:1884 -it rumqttd
```

Or you can run `rumqttd` with the custom config file by mounting the file and passing it as argument:

```bash
./build_rumqttd_docker.sh
docker run -p 1883:1883 -p 1884:1884 -v /absolute/path/to/rumqttd.toml:/rumqttd.toml -it rumqttd -c /rumqttd.toml
```

# How to use with TLS

To connect an MQTT client to rumqttd over TLS, create relevant certificates for the broker and client using [provision](https://github.com/bytebeamio/provision) as follows:
```bash
provision ca // generates ca.cert.pem and ca.key.pem
provision server --ca ca.cert.pem --cakey ca.key.pem --domain localhost // generates localhost.cert.pem and localhost.key.pem
provision client --ca ca.cert.pem --cakey ca.key.pem --device 1 --tenant a // generates 1.cert.pem and 1.key.pem
```

Update config files for rumqttd and rumqttc with the generated certificates:
```toml
[v4.2.tls]
    certpath = "path/to/localhost.cert.pem"
    keypath = "path/to/localhost.key.pem"
    capath = "path/to/ca.cert.pem"
```

You may also use [certgen](https://github.com/minio/certgen), [tls-gen](https://github.com/rabbitmq/tls-gen) or [openssl](https://www.baeldung.com/openssl-self-signed-cert) to generate self-signed certificates, though we recommend using provision.

**NOTE:** Mount the folders containing the generated tls certificates and the proper config file(with absolute paths to the certificate) to enable tls connections with rumqttd running inside docker.

## Dynamically update log filter

Log levels and filters can by dynamically updated without restarting broker.
To update the filter, we can send a POST request to `/logs` endpoint, which is exposed by our console, with new filter as plaintext in body.
For example, to get logs of rumqttd ( running locally and expose console at port 3030 ) with log level "debug", we can do:
```sh
curl -H "Content-Type: text/plain" -d "rumqttd=debug" 0.0.0.0:3030/logs
```

The general syntax for filter is:
```
target[span{field=value}]=level
```
So filter for logs of client with id "pub-001" which has occurred any any span will be `[{client_id=pub-001}]`. Know more about it [here](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/struct.EnvFilter.html#directives)

## Rate Limits and Link Congestion

`rumqttd` allows you to configure a dual-bound per-client *rate limit* (messages/min) dynamically during runtime or via defaults in `rumqttd.toml`. This limit allows you to assign specific bandwidth limits and protect the broker from congestion.

There are two bounds you can configure for a client:
* **Lower Rate (`lower_rate` limits):** If a client exceeds this rate, they are placed on "probation". It will not automatically drop them. However, if the **admin link** buffer reaches a pre-configured saturation threshold (e.g., 80% capacity), `rumqttd` detects congestion. It will identify the client on probation that is publishing the most messages and automatically disconnect it. Clients that stay within their `lower_rate` are protected from auto-disconnection during these congestion events, ensuring their traffic is guaranteed.
* **Higher Rate (`higher_rate` limits):** If a client exceeds this rate, they are immediately and forcefully disconnected from `rumqttd`, regardless of whether the system is experiencing congestion or not.

By default, the *Link Congestion* mechanism only tracks and enforces `lower_rate` probation traffic destined for the `AdminLink`.

In `rumqttd.toml` you can set global defaults:
```toml
[router]
# ...
default_lower_rate = 10.0
default_higher_rate = 50.0
congestion_threshold = 0.8
```

## Admin Link & Broker Controller

The **Admin Link** and **Broker Controller** provide programmatic interfaces to monitor and manage a running broker.

### Admin Link
The `AdminLink` is a privilege communication channel that allows you to tap into the message stream and observe data natively processed by the router without connecting via the external MQTT network. 

To create one, you typically configure it with a desired `channel_capacity`:
```rust
let mut admin_link = broker.admin_link("admin_client_id", 200)?;
```
`admin_link.recv().await` will receive all messages conforming to the subscriptions you make (`admin_link.subscribe("#")`).

*Note:* Because the Admin Link is marked internally as an administrative channel, its buffer capacity is monitored for congestion. If it fills up—specifically hitting the `congestion_threshold` config limit (defaults to `0.8` / 80%)—the robust rate-limiting disconnection algorithms will kick in to protect it. Regular remote links do not trigger the congestion auto-disconnect feature when saturated; they will just assert backpressure by dropping new data until space is available.

### Broker Controller
The `BrokerController` exposes APIs to actively manage the state of the broker and connected clients.

```rust
let controller = broker.controller();
```

With the controller, you can:
* **Disconnect clients:** `controller.disconnect_client("client_id").await?`
* **Set Rate Limits:** Protect connection traffic by altering the rate limits for specific clients at runtime:
  ```rust
  // set_rate_limits("client_id", lower_rate, higher_rate)
  controller.set_rate_limits("client_id", Some(10.0), Some(100.0)).await?
  ```
* **Read Client Information and Metrics:** `controller.get_clients().await?` returns a list of all active `ClientInfo`. Under the new rate limiting model, `ClientInfo` exposes `.message_rates`, which contains a `Vec<u32>` of up to 6 metrics—each entry capturing the exact number of messages the client transmitted in recent 1-minute buckets. Index `0` is the currently accumulating minute, and Indices `1-5` capture the previous 5 minutes of message history. You can iterate over this array to calculate rolling averages or monitor live client rates dynamically.
