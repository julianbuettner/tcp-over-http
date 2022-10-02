# 📡 TCP over HTTP

## 🥦 The Questions

- ### 🪃 What does it do?

  You can proxy TCP traffic over HTTP.

  A basic setup would be:

  ```
  [Your TCP target] <--TCP-- [Exit Node]
                                  ^
                                  |
                                HTTP
                                  |
  [Your TCP client] --TCP--> [Entry Node]
  ```

- ### 🍩 Why?

  ~~I was bored.~~

  This allows you to reach servers behind a HTTP reverse proxy.  
  Suddenly you can do SSH to a server which is behind a NGINX proxy.

  If you have for example a HTTP gateway, you can now also have
  a TCP gateway.

- ### 🍾 Why not?

  If a server only opens port 80, nobody expects you
  to tunnel through and rech the SSH server.  
  Security wise, no admin would want this tool on his/her
  server without him/her knowing.

## :) installation

- get yourself a rust toolchain via rustup https://www.rust-lang.org/tools/install
- ```bash
  cargo install --locked --git https://github.com/julianbuettner/tcp-over-http
  ```

## 🎺 Usage

Replace `tcp-over-http` by `cargo run --release --`
if you have not installed the binary.

```bash
tcp-over-http --help

# Start our exit node to reach our SSH server (default listen localhost:8080)
tcp-over-http exit --help
tcp-over-http exit --target-host localhost --target-port 22

# Start our entry node (default listen localhost:1415)
tcp-over-http entry --help
tcp-over-http entry --target-url http://localhost:8080/

# Test it
ssh localhost -p 1415
```

## ⌚️ Performance

This package is not optimized for stability ~~or speed~~.

_Setup_

```bash
# Terminal 0 - Netcat listening
nc -l 1234 > /dev/null

# Terminal 1 - Exit Node
tcp-over-http exit --target-host locahost --target-port 1234

# Terminal 2 - Entry Node
tcp-over-http entry --target-url http://localhost:8080/

# Terminal 3 - Sending \0 data
# Using pipeviewer (pv) to see current data rate
time cat /dev/zero | pv | nc localhost 1415
```

### 🏅 Result: $$$$MiB/s
