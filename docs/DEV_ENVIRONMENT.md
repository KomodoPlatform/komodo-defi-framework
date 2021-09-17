# Setting up the dev environment for AtomicDEX-API to run full tests suite

1. Install docker.
2. Download ZCash params files: [Windows](https://github.com/KomodoPlatform/komodo/blob/master/zcutil/fetch-params.bat), [Unix/Linux](https://github.com/KomodoPlatform/komodo/blob/master/zcutil/fetch-params.sh)
3. Create `.env.client` file with the following content
```
PASSPHRASE=spice describe gravity federal blast come thank unfair canal monkey style afraid
```
4. Create `.env.seed` file with the following content
```
PASSPHRASE=also shoot benefit prefer juice shell elder veteran woman mimic image kidney
```
5. Try `cargo test --features native --all -- --test-threads=16`.

PS If you notice that this guide is outdated, please submit a PR.