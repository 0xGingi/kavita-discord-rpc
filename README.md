# kavita-discord-rpc

![image](https://github.com/user-attachments/assets/b5efcc84-07e2-4849-b737-bf4b4a006c07)

![image](https://github.com/user-attachments/assets/a5c56d2a-2015-456c-948a-a08b769cd54c)


Displays what you're reading on kavita on discord!

Note: You must run this on a system with discord open, but this will work on any device you read on! (3rd party client support is unknown)

Bug Note: Currently this may not work if your client and server are on different timezones! I have a PR to fix this on kavita as UTC on both sides would fix https://github.com/Kareadita/Kavita/pull/3640
This is temporarily solved with this branch + my docker image with the api change https://github.com/0xGingi/kavita-discord-rpc/tree/readdateutc

## Install

1. grab your linux or windows binary from: https://github.com/0xGingi/kavita-discord-rpc/releases
2. copy the config.example.json and rename it as config.json in the same folder as your binary
3. Modify the config.json with your info
4. run

## Docker (Only works on Linux - Discord must be installed on the system)
Note: If using windows, this may work via WSL2, Discord must also be installed via WSL2 and open

### Docker

```
docker run -d \
  -v ./config:/app/config \
  -v /run/user/1000/discord-ipc-0:/run/user/1000/discord-ipc-0 \
  -e XDG_RUNTIME_DIR=/run/user/1000 \
  -e TZ=America/New_York \
  --restart unless-stopped \
  --name kavita-discord-rpc \
  0xgingi/kavita-discord-rpc:latest
```

Edit your timezone and ensure you've created a config folder with config.json in it.

### Docker Compose

1. clone the repo
```
git clone https://github.com/0xGingi/kavita-discord-rpc
cd kavita-discord-rpc
```
2. Rename config/config-example.json to config/config.json and modify it
3. Modify docker-compose.yml -TZ with your kavita server's timezone
4. Run
```
docker compose up -d
```

## Build
```
git clone https://github.com/0xgingi/kavita-discord-rpc
cd kavita-discord-rpc
cargo build --release
```
