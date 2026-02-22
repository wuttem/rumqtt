# External Authentication in rumqttd

The `rumqttd` broker allows you to define custom logic for authenticating connecting MQTT clients. This allows you to verify credentials against external systems, such as a PostgreSQL database, an LDAP server, a REST API, or simply by dynamically parsing certificate information across multiple tenants.

This document accompanies the `external_auth.rs` example and explains how this mechanism works.

## Actor-based Auth Example

Although you can provide any async closure that satisfies the `AuthHandler` signature, spinning up an **Auth Actor** is highly recommended for complex authentication workflows. 

### Why an Actor?
When thousands of clients connect simultaneously, directly querying a database per connection can overwhelm both the asynchronous runtime executor and the backend database. By placing an actor on an asynchronous event channel (like `flume` or `tokio::sync::mpsc`):
- You effectively decouple the authentication pool boundaries from the main broker thread tasks.
- The actor can implement advanced optimizations like batching SQL queries, caching responses, or rate-limiting authentication requests per IP.

### The Callback Interface
The `set_auth_handler` expects an async callback that resolves to `Result<Option<ClientInfo>, String>`. 
```rust
server.set_auth_handler(
    move |client_id: String, username: String, password: String, common_name: String, organization: String| {
        async move {
            // Your custom authentication logic goes here!
            // Return Ok(None) to proceed with standard rumqttd mapping,
            // or Ok(Some(ClientInfo)) to dynamically override client parameters.
        }
    }
);
```

## Certificate Information: `common_name` and `organization`

If a client connects to a server endpoint configured with **mTLS (Mutual TLS)**, `rumqttd` automatically verifies their certificate and attempts to extract their subject information. This requires the `verify-client-cert` feature flag to be enabled.

During the TLS handshake, `rumqttd` uses `x509_parser` to inspect the client's X.509 certificate. From the `Subject` field of the certificate:
1. **`common_name` (CN)**: Extracted and often used as the default MQTT `client_id` boundaries if the client does not specify one themselves.
2. **`organization` (O)**: Extracted and used by `rumqttd` as a strict **Tenant ID**. 

> **Note**: These names cannot contain a period (`.`) at the moment due to rumqttd's strict tenant router segmentation.

## Multi-Tenant Certificate Injection

A multi-tenant architecture often requires clients to be grouped into isolated segments (Tenants), while securing their connectivity using certificates. Here is exactly how to do that in `rumqttd`:

### Where do these certificates go?
In your `rumqttd.toml`, you specify a `capath` inside your TLS server settings.

```toml
[v4.1.tls]
certpath = "broker.crt"
keypath = "broker.key"
capath = "ca_certs" # <- Can be either a single Root CA file, or a directory!
```

If `capath` is a directory, `rumqttd` will automatically iterate through all files within it and register every valid `.pem` / `.crt` file it finds into its root trust store! This allows each Tenant to supply and use their own distinct CA root certificate without relying on a central authority.

### Supporting Multiple Organizations
The `capath` acts as a broad "Trust Anchor" configuration for the rustls acceptor limit. All client certificates signed directly (or via an intermediate) by this `root_ca.crt` will be fundamentally **cryptographically trusted** by the broker.

Because `rumqttd` extracts the `Organization (O)` attribute automatically from each validated client certificate to inject into the authentication flow:
1. **The Infrastructure Team** spins up a single `root_ca.crt`.
2. **Tenant A** connects using a certificate signed by the Root CA with `O=TenantA`. `rumqttd` routes this client inside the `TenantA` partition. 
3. **Tenant B** connects using a certificate signed by the same Root CA with `O=TenantB`. `rumqttd` routes this client inside the `TenantB` partition.

This ensures **Zero-Trust Multi-Tenancy**. Using the external `AuthActor` logic, you can then also enforce permissions (e.g. denying Tenant B if their database billing account is frozen) right at the `set_auth_handler` closure.
