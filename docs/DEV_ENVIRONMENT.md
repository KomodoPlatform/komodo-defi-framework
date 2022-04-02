# Setting up the dev environment for AtomicDEX-API to run full tests suite

1. Install Docker or Podman.
2. Download ZCash params files: [Windows](https://github.com/KomodoPlatform/komodo/blob/master/zcutil/fetch-params.bat), [Unix/Linux](https://github.com/KomodoPlatform/komodo/blob/master/zcutil/fetch-params.sh)
3. Create `.env.client` file with the following content
```
PASSPHRASE=spice describe gravity federal blast come thank unfair canal monkey style afraid
```
4. Create `.env.seed` file with the following content
```
PASSPHRASE=also shoot benefit prefer juice shell elder veteran woman mimic image kidney
```
5. MacOS specific: run script (required after each reboot)
```shell
#!/bin/bash
for ((i=2;i<256;i++))
do
    sudo ifconfig lo0 alias 127.0.0.$i up
done
```
Please note that you have to run it again after each reboot  
6. Linux specific:
- for Docker users:
```
sudo groupadd docker
sudo usermod -aG docker $USER
```
- for Podman users:
```
sudo ln -s $(which podman) /usr/bin/docker
```

7. Try `cargo test --features native --all -- --test-threads=16`.

PS If you notice that this guide is outdated, please submit a PR.
