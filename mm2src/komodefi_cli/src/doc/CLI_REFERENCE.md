`komodefi-cli` provides a CLI interface and facilitates interoperating to komodo defi platform via the `mm2` service. It's a multi-purpose utility that facilitates using komodo platform to be used as a multi-chain Wallet and DEX at the same time.

## Build

The `komodefi-cli` binary file can be built in the root of the project using the following commands:

```sh
export PATH=${PATH}:$(pwd)/bin
cargo build --manifest-path=mm2src/komodefi_cli/Cargo.toml --out-dir bin -Z unstable-options
```

Now `komodefi-cli` is built and available in the `bin` directory and can be called, as is done in the current reference examples.

## Manage mm2 service

`komodefi-cli`  should be configured in a proper way to be able to connect to and interract with the running `mm2` service via the Komodo RPC API. The `mm2` can be started manually or by using the `komodefi-cli` that facilitates configuring and managing it as a process.

### Setting the Default CLI Root
You can customize the default root directory for `komodefi-cli` by setting the `KOMODEFI_CLI_ROOT` environment variable. This allows you to specify a location for saving configuration files, binaries, and other related data.
 
```sh
export KOMODEFI_CLI_ROOT=$HOME/komodefi-cli
komodefi-cli mm2 download
```

### init

The `init` command is aiming to facilitate creating of the `MM2.json` configuration file and getting the `coins` configuration. The `komodefi-cli` implements it in an interactive step-by-step mode and produces `MM2.json` as a result. `coins` configuration is got from the https://raw.githubusercontent.com/KomodoPlatform/coins/master/coins. The `init` also provides alternative paths setting by additional options.

```sh
komodefi-cli init --help  
Initialize a predefined coin set and configuration to start mm2 instance with  
  
Usage: komodefi-cli init [OPTIONS]  
  
Options:  
     --mm-coins-path <MM_COINS_PATH>  Coin set file path [default: coins] [aliases: coins]  
     --mm-conf-path <MM_CONF_PATH>    mm2 configuration file path [default: MM2.json] [aliases: conf]  
 -h, --help                           Print help
```

**Example**:

```sh
RUST_LOG=error komodefi-cli init  
> What is the network `mm2` is going to be a part, netid: 7777  
> What is the seed phrase: upgrade hunt engage mountain cheap hood attitude bleak flag wild feature aim  
> Allow weak password: No  
> What is the rpc_password: D6~$jETp  
> What is dbdir None  
> What is rpcip: None  
> What is the rpcport: None  
> What is rpc_local_only:    
> Use secure connection for rpc: No  
> What is i_am_a_seed: 
? What is the next seednode: Tap enter to skip    
[Optional. If operating on a test or private netID, the IP address of at least one seed node is required (on the main network, these are already hardcoded)]
...
```

resulting `MM2.json`:

```json
{  
    "gui": "komodefi-cli",  
    "netid": 7777,  
    "rpc_password": "D6~$jETp",  
    "passphrase": "upgrade hunt engage mountain cheap hood attitude bleak flag wild feature aim",  
    "allow_weak_password": false,
    "rpcip": "127.0.0.1",
    "rpcport": 7783,
    "secure_conn": false
}
```

### mm2 download
Download the latest version of `mm2` binary and save to `KOMODEFI_CLI_ROOT` dir.

Usage:
```sh
komodefi-cli mm2 download
```

### mm2 start

The `start` command is used to start `mm2` instance. Options are used to be set as an environment variables of the `mm2 `: `MM_CONF_PATH`, `MM_COINS_PATH` and `MM_LOG` accordingly.

```sh
komodefi-cli mm2 start -help  
Start mm2 instance  
  
Usage: komodefi-cli mm2 start [OPTIONS]  
  
Options:  
     --mm-conf-path <MM_CONF_PATH>    mm2 configuration file path [aliases: conf]  
     --mm-coins-path <MM_COINS_PATH>  Coin set file path [aliases: coins]  
     --mm-log <MM_LOG>                Log file path [aliases: log]  
 -h, --help                           Print help
```

**Example**:

```sh
komodefi-cli mm2 start --mm-log mm2.log  
Set env MM_LOG as: mm2.log  
Started child process: "mm2", pid: 459264
```
### mm2 status

The `check` command gets the state of the running `mm2` instance

```sh
komodefi-cli mm2 status --help  
Check if mm2 is running  
  
Usage: komodefi-cli mm2 status  
  
Options:  
 -h, --help  Print help
```

**Example**:

```sh
komodefi-cli mm2 status    
Found mm2 is running, pid: 459264
```

```sh
komodefi-cli mm2 status    
Process not found: mm2
```
### mm2 version

The `version` command requests the version of `mm2` using the [`version` RPC API method](https://developers.komodoplatform.com/basic-docs/atomicdex-api-legacy/version.html)

```sh
komodefi-cli mm2 version --help  
Get version of intermediary mm2 service  
  
Usage: komodefi-cli mm2 version  
  
Options:  
 -h, --help  Print help
```

**Example**:

```sh
komodefi-cli mm2 version  
Version: 1.0.6-beta_9466e53c6  
Datetime: 2023-09-11T16:19:16+05:00
```

### mm2 stop

The `stop` command requests stopping of mm2 using the [`stop` RPC API method](https://developers.komodoplatform.com/basic-docs/atomicdex-api-legacy/stop.html)

```sh
komodefi-cli mm2 stop --help  
Stop mm2 using API  
  
Usage: komodefi-cli mm2 stop  
  
Options:  
 -h, --help  Print help
```

**Example**:

```sh
komodefi-cli mm2 stop    
Sending stop command  
Service stopped: Success
```

### mm2 kill

The `kill` command kills the process of `mm2` if it is currently running

```sh
komodefi-cli mm2 kill --help  
Kill mm2 process  
  
Usage: komodefi-cli mm2 kill  
  
Options:  
 -h, --help  Print help
```

**Example:**

```sh
komodefi-cli mm2 kill    
Process killed: mm2:505606
```

## Configuring `komodefi-cli`

Config commands are `set` and `get`.  These are aimed to set and get password and uri to connect to and be 
granted to request methods from the running `mm2` instance. Password is setting in an interactive mode. The certain path of the configuration depends on the operating system the `komodefi-cli` is running on and could be different.

```sh
komodefi-cli config set --help  
Set komodo komodefi cli configuration  
  
Usage: 
- komodefi-cli config set <--password|--uri <URI>>
- komodefi-cli config set --from-path (sets config from MM2.json file automatically)
  
Options:  
 -p, --password   Set if you are going to set up a password  
 -u, --uri <URI>  KomoDeFi RPC API Uri. http://localhost:7783 [aliases: url]  
 -h, --help       Print help
```

**Examples:**

Setting configuration:

```sh
komodefi-cli config set -u https://localhost:7783 -p  
? Enter RPC API password:  
? Confirmation:
```

resulting `komodefi_cfg.json`:

```
cat ~/.config/komodefi-cli/komodefi_cfg.json    
{  
 "rpc_password": "D6~$jETp",  
 "rpc_uri": "https://localhost:7783"  
}
```

Getting configuration:

```sh
release/komodefi-cli config get   
mm2 RPC URL: https://localhost:7783  
mm2 RPC password: *************
```

```sh
release/komodefi-cli config get --unhide 
mm2 RPC URL: https://localhost:7783  
mm2 RPC password: SomeVeryStrongPassword
```

## Setting up coin index

To use the komodo platform as both a multi-chain wallet and DEX, there are commands to include and exclude certain coins from the wallet index. RPC API methods can mainly be requested to be used with only enabled coins.

```sh
komodefi-cli coin  
Coin commands: enable, disable etc.  
  
Usage: komodefi-cli coin <COMMAND>  
  
Commands:  
 enable               Put a coin to the trading index  
 disable              Deactivates enabled coin and also cancels all active orders that use the selected coin.  
 get-enabled          List activated coins [aliases: enabled]  
 set-required-conf    Set the number of confirmations to wait for the selected coin [aliases: set-conf]  
 set-required-nota    Whether to wait for a dPoW notarization of the given atomic swap transactions [aliases: set-nota]  
 coins-to-kick-start  Return the coins that should be activated to continue the interrupted swaps [aliases: to-kick]  
 help                 Print this message or the help of the given subcommand(s)  
  
Options:  
 -h, --help  Print help
```
### coin enable

The `enable` command includes the given coin in the komodo wallet index. Depending on the given coin the different RPC API method could be requested. For the [ZHTLC related method](https://developers.komodoplatform.com/basic-docs/atomicdex-api-20-dev/zhtlc_coins.html#task-enable-z-coin-init) - `--keep-progress` option is designed to request the status of the enabling task every N seconds. The `--tx-history` option overrides the predefined setting to make it able to request transactions history for the given coin.

```sh
komodefi-cli coin enable --help  
Put a coin to the trading index  
  
Usage: komodefi-cli coin enable [OPTIONS] <COIN>  
  
Arguments:  
 <COIN>  Coin to be included into the trading index  
  
Options:  
 -k, --keep-progress <KEEP_PROGRESS>  Whether to keep progress on task based commands [default: 0] [aliases: track, keep, progress]  
 -H, --tx-history                     Whether to save tx history for the coin [aliases: history]  
 -h, --help                           Print help
```

*Notice: To override required conf and nota `set-required-conf` and `set-required-nota` commands could be used*

**Examples**

Enabling `DOC` using the ([legacy method](https://developers.komodoplatform.com/basic-docs/atomicdex-api-legacy/coin_activation.html))

```sh
komodefi-cli coin enable --tx-history DOC  
Enabling coin: DOC  
coin: DOC  
address: RNfMoWT47LWht7wwixJdx9QP51mkakLVsU  
balance: 0  
unspendable_balance: 0  
required_confirmations: 1  
requires_notarization: No  
mature_confirmations: 100
```

Enabling `tBCH` ([v2.0 method](https://developers.komodoplatform.com/basic-docs/atomicdex-api-20/enable_bch_with_tokens.html))

```sh
komodefi-cli coin enable tBCH  
Enabling coin: tBCH  
current_block: 1570304  
bch_addresses_infos:    
│ address, pubkey                                   │ method │ balance(sp,unsp) │ tickers │  
│ bchtest:qzvnf6l254ktt97fx657mqszmrvh5znsqvs26sxf6t│ Iguana │ 0.05:0           │ none    │  
│ 02264fcd9401d797c50fe2f1c7d5fe09bbc10f3838c1d8d...│        │                  │         │  
  
slp_addresses_infos:    
│ address, pubkey                                   │ method │ balance(sp,unsp) │ tickers │  
│ slptest:qzvnf6l254ktt97fx657mqszmrvh5znsqvt7atu7gk│ Iguana │ {}               │ none    │  
│ 02264fcd9401d797c50fe2f1c7d5fe09bbc10f3838c1d8d...│        │                  │         │
```

Enable ZHTLC `ZOMBIE` COIN ([`task_enable_z_coin_init` method](https://developers.komodoplatform.com/basic-docs/atomicdex-api-20-dev/zhtlc_coins.html#task-enable-z-coin-init))

```sh
komodefi-cli coin enable ZOMBIE --track 5  
Enabling coin: ZOMBIE  
Enabling zcoin started, task_id: 0  
Getting enable zcoin task status  
In progress: Activating coin  
In progress: Activating coin  
In progress: Activating coin  
In progress: Activating coin  
In progress: Updating block cache, current_scanned_block: 58980, latest_block: 77153  
In progress: Building wallet db, current_scanned_block: 56397, latest_block: 77153  
In progress: Building wallet db, current_scanned_block: 60397, latest_block: 77153  
In progress: Building wallet db, current_scanned_block: 65397, latest_block: 77153  
In progress: Building wallet db, current_scanned_block: 69397, latest_block: 77153  
In progress: Building wallet db, current_scanned_block: 73397, latest_block: 77153  
status: OK  
current_block: 77154  
ticker: ZOMBIE  
iguana wallet:    
            address: zs1r0fzx9unydgfty74z5d4qkvjyaky0n73ms4cvhttj4234s6rf0hfju5faf6a5nzlwv5qgrr0pen  
            balance: 50.77:0
```

```sh
komodefi-cli coin enable ZOMBIE  
Enabling coin: ZOMBIE  
Enabling zcoin started, task_id: 1

komodefi-cli task status zcoin 1  
Getting enable zcoin task status  
status: OK  
current_block: 77154  
ticker: ZOMBIE  
iguana wallet:    
            address: zs1r0fzx9unydgfty74z5d4qkvjyaky0n73ms4cvhttj4234s6rf0hfju5faf6a5nzlwv5qgrr0pen  
            balance: 50.77:0
```
### coin disable

The `disable` command excludes the given coin from the komodo wallet index using the [disable_coin RPC API method](https://developers.komodoplatform.com/basic-docs/atomicdex-api-legacy/disable_coin.html).

```sh
komodefi-cli coin disable --help  
Deactivates enabled coin and also cancels all active orders that use the selected coin.  
  
Usage: komodefi-cli coin disable <COIN>  
  
Arguments:  
 <COIN>  Coin to disable  
  
Options:  
 -h, --help  Print help
```

**Example:**

```sh
Disabling coin: DOC  
coin: DOC  
cancelled_orders: 8339efe3-ee0a-436c-ad6a-88ef8130ba2b  
passivized: false
```
### coin get-enabled (enabled)

The `get-enabled` command lists coins that are already in the wallet index using the [get_enabled_coins RPC API method](https://developers.komodoplatform.com/basic-docs/atomicdex-api-legacy/get_enabled_coins.html). *Alias: `enabled`*

```sh
komodefi-cli coin get-enabled --help  
List activated coins  
  
Usage: komodefi-cli coin get-enabled  
  
Options:  
 -h, --help  Print help
```

**Example**:

```
komodefi-cli coin enabled  
Getting list of enabled coins ...  
Ticker   Address  
ZOMBIE   zs1r0fzx9unydgfty74z5d4qkvjyaky0n73ms4cvhttj4234s6rf0hfju5faf6a5nzlwv5qgrr0pen  
tBCH     bchtest:qzvnf6l254ktt97fx657mqszmrvh5znsqvs26sxf6t  
MARTY    RPFGrvJWjSYN4qYvcXsECW1HoHbvQjowZM  
DOC      RPFGrvJWjSYN4qYvcXsECW1HoHbvQjowZM
```
### coin coins-to-kick-start (to-kick)

The `coins-to-kick-start` command lists coins that are involved in swaps that are not done and needed to be anbled to complete them. This command is proceed using the [`coins_needed_to_kick_start` RPC API method](https://developers.komodoplatform.com/basic-docs/atomicdex-api-legacy/coins_needed_for_kick_start.html) *Alias: `to-kick`*

```sh
komodefi-cli coin to-kick --help  
Return the coins that should be activated to continue the interrupted swaps  
  
Usage: komodefi-cli coin coins-to-kick-start  
  
Options:  
 -h, --help  Print help
 ```

**Example:**

```sh
komodefi-cli coin to-kick    
Getting coins needed for kickstart  
coins: RICK, MORTY
```
### coin set-required-conf (set-conf)

The `set-required-conf` command temporary overrides the predefined `required-conf` setting of the given coin using the [`set_required_confirmations` RPC API method](https://developers.komodoplatform.com/basic-docs/atomicdex-api-legacy/set_required_confirmations.html) *Alias: `set-conf`*

```sh
komodefi-cli coin set-conf --help  
Set the number of confirmations to wait for the selected coin  
  
Usage: komodefi-cli coin set-required-conf <COIN> <CONFIRMATIONS>  
  
Arguments:  
 <COIN>           Ticker of the selected coin  
 <CONFIRMATIONS>  Number of confirmations to require [aliases: conf]  
  
Options:  
 -h, --help  Print help
```

**Example:**

```sh
komodefi-cli coin set-conf DOC 3  
Setting required confirmations: DOC, confirmations: 3  
coin: DOC  
confirmations: 3
```

### coin set-required-nota (set-nota)

The `set-required-nota` command temporary overrides the predefined `required-nota` setting of the given coin using the [`set_required_notarization` RPC API method](https://developers.komodoplatform.com/basic-docs/atomicdex-api-legacy/set_requires_notarization.html). *Alias: `set-nota`*

```sh
komodefi-cli coin set-nota --help  
Whether to wait for a dPoW notarization of the given atomic swap transactions  
  
Usage: komodefi-cli coin set-required-nota [OPTIONS] <COIN>  
  
Arguments:  
 <COIN>  Ticker of the selected coin  
  
Options:  
 -n, --requires-notarization  Whether the node should wait for dPoW notarization of atomic swap transactions [aliases: requires-nota]  
 -h, --help                   Print help
```

**Examples:**

Set `requried_nota`:

```sh
komodefi-cli coin set-nota --requires-nota DOC  
Setting required nota: DOC, requires_nota: true  
coin: DOC  
requires_notarization: true
```

Reset `required_nota`:

```sh
komodefi-cli coin set-nota DOC  
Setting required nota: DOC, requires_nota: false  
coin: DOC  
requires_notarization: false
```

## Wallet commands

The further group of commands are aimed to provide base multi-chain wallet functionality. It includes commands like balance, withdraw, tx-history etc.

```sh
komodefi-cli wallet  
Wallet commands: balance, withdraw etc.  
  
Usage: komodefi-cli wallet <COMMAND>  
  
Commands:  
 my-balance            Get coin balance [aliases: balance]  
 withdraw              Generates, signs, and returns a transaction that transfers the amount of coin to the address indicated in the to argument  
 send-raw-transaction  Broadcasts the transaction to the network of the given coin [aliases: send-raw, send]  
 get-raw-transaction   Returns the full signed raw transaction hex for any transaction that is confirmed or within the mempool [aliases: get-raw, raw-tx, get]  
 tx-history            Returns the blockchain transactions involving the Komodo DeFi Framework node's coin address [aliases: history]  
 show-priv-key         Returns the private key of the specified coin in a format compatible with coin wallets [aliases: private, private-key]  
 validate-address      Checks if an input string is a valid address of the specified coin [aliases: validate]  
 kmd-rewards-info      Informs about the active user rewards that can be claimed by an address's unspent outputs [aliases: rewards]  
 convert-address       Converts an input address to a specified address format [aliases: convert]  
 convert-utxo-address  Takes a UTXO address as input, and returns the equivalent address for another UTXO coin (e.g. from BTC address to RVN address) [aliases: convert-utxo]  
 get-public-key        Returns the compressed secp256k1 pubkey corresponding to the user's seed phrase [aliases: get-public, public-key, public]  
 get-public-key-hash   Returns the RIPEMD-160 hash version of your public key [aliases: pubkey-hash, hash, pubhash]  
 help                  Print this message or the help of the given subcommand(s)  
  
Options:  
 -h, --help  Print help
```

### wallet my-balance (balace)

The `my-balance` command gets balance of the given coin using the [`my_balance` RPC API method](https://developers.komodoplatform.com/basic-docs/atomicdex-api-legacy/my_balance.html). *Alias: `balance`*.

```sh
komodefi-cli wallet balance --help  
Get coin balance  
  
Usage: komodefi-cli wallet my-balance <COIN>  
  
Arguments:  
 <COIN>  Coin to get balance  
  
Options:  
 -h, --help  Print help
```

**Examples:**

```sh
komodefi-cli wallet balance DOC  
Getting balance, coin: DOC  
coin: DOC  
balance: 949832.3746621  
unspendable: 0  
address: RPFGrvJWjSYN4qYvcXsECW1HoHbvQjowZM
```

### wallet withdraw

The `withdraw` command return a signed transaction binary data that is supposed to be sent with `send-raw-transaction`. The withdrawal is done using the [`withdraw` RPC API method](https://developers.komodoplatform.com/basic-docs/atomicdex-api-legacy/withdraw.html). `--amount` and  `--max` options can not be used together. `--fee` options provides setting up custom fee for utxo,  eth, qrc and cosmos. `--derivation-path` (*currently not supported*) and (`--hd-account-id`, `--hd-is-change`, `--hd-address-index`) are parameters that allow withdrawing from the certain [bip32 compatible derived address ](https://github.com/bitcoin/bips/blob/master/bip-0032.mediawiki) if `enable_hd` option is enabled as a part of current `mm2` configuration. `--bare-output` option prevents an extra output to be able to call methods in sequentially.

```sh
release/komodefi-cli wallet withdraw --help  
Generates, signs, and returns a transaction that transfers the amount of coin to the address indicated in the to argument  
  
Usage: komodefi-cli wallet withdraw [OPTIONS] <COIN> <TO>  
  
Arguments:  
 <COIN>  Coin the user desires to withdraw  
 <TO>    Address the user desires to withdraw to  
  
Options:  
 -M, --max  
         Withdraw the maximum available amount  
 -a, --amount <AMOUNT>  
         Amount the user desires to withdraw  
 -f, --fee <FEE>  
         Transaction fee [possible-values: <utxo-fixed:amount|utxo-per-kbyte:amount|eth:gas_price:gas|qrc:gas_limit:gas_price|cosmos:gas_limit:gas_price>]  
     --derivation-path <DERIVATION_PATH>  
         Derivation path to determine the source of the derived value in more detail  
     --hd-account-id <HD_ACCOUNT_ID>  
         Account index of the same crypto currency  
     --hd-is-change <HD_IS_CHANGE>  
         Is change [possible values: true, false]  
     --hd-address-index <HD_ADDRESS_INDEX>  
         An incremental address index for the account  
 -b, --bare-output  
         Whether to output only tx_hex [aliases: bare]  
 -h, --help  
         Print help
```

**Example**:

```sh
komodefi-cli wallet withdraw DOC RQvvz29iHpn4iuHeN7qFXnLbc1eh31nRKY --amount 1  
Getting withdraw tx_hex  
coin: DOC  
from: RPFGrvJWjSYN4qYvcXsECW1HoHbvQjowZM  
to: RQvvz29iHpn4iuHeN7qFXnLbc1eh31nRKY  
total_amount: 949831.3746521  
spent_by_me: 949831.3746521  
received_by_me: 949830.3746421  
my_balance_change: -1.00001  
block_height: 0  
timestamp: 23-09-13 14:32:29  
fee_details: {"type":"Utxo","coin":"DOC","amount":"0.00001"}  
internal_id:    
transaction_type: "StandardTransfer"  
tx_hash: 04bfc0470a07ab1390dffb98164543e368d100101020a4f9b20bf3f8193c2ea0  
tx_hex: 0400008085202f8901d4a7ffce25cdc0ca0cbfa23363e1159195a3fafd742bf8bfc6ba0f50a51b7160010000006a4730440220590868b953be9c655ebd97cf750fbd669171006848900c5ea9d0e151a1dfb28b02203b6ca6955d2f558c4b5fdbf5cc1a149cbe6a57685be34d9f7594b140ad  
698765012102264fcd9401d797c50fe2f1c7d5fe09bbc10f3838c1d8d6f793061fa5f38b2b4dffffffff0200e1f505000000001976a914abad17c49173537a07e39d2c94dcfe64645b1ad488ac922e35f6625600001976a9149934ebeaa56cb597c936a9ed8202d8d97a0a700388acfdc70165000000  
000000000000000000000000
```

Bare output:

```sh
RUST_LOG=error komodefi-cli wallet withdraw --amount 1 --fee utxo-fixed:0.01  DOC RQvvz29iHpn4iuHeN7qFXnLbc1eh31nRKY -b  
0400008085202f8901d4a7ffce25cdc0ca0cbfa23363e1159195a3fafd742bf8bfc6ba0f50a51b7160010000006b483045022100a9bb524d413fea1a417a5d9ac1465c8995eadd642704c003ab5a2be22b533546022044d3f33582773254930f5054afa89d9ce085369a1b3c2c47302ab5dbf17cfa13  
012102264fcd9401d797c50fe2f1c7d5fe09bbc10f3838c1d8d6f793061fa5f38b2b4dffffffff0200e1f505000000001976a914abad17c49173537a07e39d2c94dcfe64645b1ad488ac3af025f6625600001976a9149934ebeaa56cb597c936a9ed8202d8d97a0a700388ac7c850165000000000000  
000000000000000000
```

### wallet send-raw-transaction (send-raw, send)

The `send-raw-transaction` command sends a raw transaction to a given coin network using the [`send_raw_transaction` RPC API method](https://developers.komodoplatform.com/basic-docs/atomicdex-api-legacy/send_raw_transaction.html) The `--coin` and `--tx-hex` options specify the coin and transaction data, accordingly. The optional `--bare-output` (`bare`) parameter is intended to suppress additional output. *Aliases: `send-raw`, `send`*

```sh
komodefi-cli wallet send -h  
Broadcasts the transaction to the network of selected coin  
  
Usage: komodefi-cli wallet send-raw-transaction [OPTIONS] --coin <COIN> --tx-hex <TX_HEX>  
  
Options:  
 -c, --coin <COIN>      Name of the coin network on which to broadcast the transaction  
 -t, --tx-hex <TX_HEX>  Transaction bytes in hexadecimal format;  
 -b, --bare-output      Whether to output only tx_hash [aliases: bare]  
 -h, --help             Print help
```

**Example:**

```sh
komodefi-cli wallet send --coin DOC --tx-hex 0400008085202f8901d4a7ffce25cdc0ca0cbfa23363e1159195a3fafd742bf8bfc6ba0f50a51b7160010000006a47304402200ddbcda20d288d1c6075d01bb55158e18353f0e5  
e9789cddfb572ad0429e96f7022040d2c66dbab960c5746ceb16aec7d6f0f85364d939377cbfdc060ad8f24dd0be012102264fcd9401d797c50fe2f1c7d5fe09bbc10f3838c1d8d6f793061fa5f38b2b4dffffffff0200e1f505000000001976a914abad17c49173537a07e39d2c94dcfe64645b1ad4  
88ac922e35f6625600001976a9149934ebeaa56cb597c936a9ed8202d8d97a0a700388ac3acb0165000000000000000000000000000000  
Sending raw transaction  
tx_hash: 58aca0e311513974e467187026b873621c68a4ddedb16175c558d3886dbc3dc4
```

Calling methods sequentially:

```sh
komodefi-cli wallet send -b --coin DOC --tx-hex  $(RUST_LOG=error komodefi-cli wallet withdraw DOC RQvvz29iHpn4iuHeN7qFXnLbc1eh31nRKY --amount 1 -b)
Sending raw transaction  
26f4f904c60d5d0a410fa6e00fb540e19e983f52045b2d29f97d046c58e9ecd0
```

This transaction could be found [in the DOC coin explorer](https://doc.explorer.dexstats.info/tx/26f4f904c60d5d0a410fa6e00fb540e19e983f52045b2d29f97d046c58e9ecd0)
### wallet get-raw-transaction (get-raw, raw-tx, get)

The `get-raw-transaction`  get raw transaction data from the given coin's network using the [`get_raw_transaction` RPC API method](https://developers.komodoplatform.com/basic-docs/atomicdex-api-20/get_raw_transaction.html). *Aliases: `get-raw`, `raw-tx`, `get`*

```sh
komodefi-cli wallet get -h  
Returns the full signed raw transaction hex for any transaction that is confirmed or within the mempool  
  
Usage: komodefi-cli wallet get-raw-transaction [OPTIONS] --coin <COIN> --tx-hash <TX_HASH>  
  
Options:  
 -c, --coin <COIN>        Coin the user desires to request for the transaction  
 -H, --tx-hash <TX_HASH>  Hash of the transaction [aliases: hash]  
 -b, --bare-output        Whether to output only tx_hex [aliases: bare]  
 -h, --help               Print help
```

**Example:**

```sh
komodefi-cli wallet get-raw-transaction -c DOC -H 26f4f904c60d5d0a410fa6e00fb540e19e983f52045b2d29f97d046c58e9ecd0 -b  
Getting raw transaction of coin: DOC, hash: 26f4f904c60d5d0a410fa6e00fb540e19e983f52045b2d29f97d046c58e9ecd0  
0400008085202f8901b3d8bfad51d9d44ba1d721180596cfe7e2f38969a582a3d358d8bc6cb80cb918010000006a47304402201fc52ae1078a0fd12b79b76170165919a8c36c6aae978e2ed8f204998f46095c02205e5b6dfe9c118b3acef2c28a7454e9d153fbdd89d0f6abc24fc2e2b966486fff01  
2102264fcd9401d797c50fe2f1c7d5fe09bbc10f3838c1d8d6f793061fa5f38b2b4dffffffff0200e1f505000000001976a914abad17c49173537a07e39d2c94dcfe64645b1ad488acc26449ea625600001976a9149934ebeaa56cb597c936a9ed8202d8d97a0a700388ac01d1016500000000000000  
0000000000000000
```

### wallet tx-history (history)

If the given coin was enabled with `--tx-history` it's getting possible to request history of transactions using the [`my_tx_history` RPC API method](https://developers.komodoplatform.com/basic-docs/atomicdex-api-legacy/my_tx_history.html).  Requesting transaction set could be limited with `--limit` option or  not if `--max` is taking place. The resulting transactions are organized into pages, there is the `--page-number` (`--page`) options that allows to select the certain one. If one of `--from-tx-hash` or `--from-tx-id` is taking place the resulting set will be restricted with the given hash or id accordingly.
*Alias: history*

```sh
komodefi-cli wallet tx-history --help  
Returns the blockchain transactions involving the Komodo DeFi Framework node's coin address  
  
Usage: komodefi-cli wallet tx-history [OPTIONS] <--limit <LIMIT>|--max> <COIN>  
  
Arguments:  
 <COIN>  The name of the coin for the history request  
  
Options:  
 -l, --limit <LIMIT>                Limits the number of returned transactions  
 -m, --max                          Whether to return all available records  
 -H, --from-tx-hash <FROM_TX_HASH>  Skips records until it reaches this ID, skipping the from_id as well  
 -i, --from-tx-id <FROM_TX_ID>      For zcoin compatibility, skips records until it reaches this ID, skipping the from_id as well  
 -p, --page-number <PAGE_NUMBER>    The name of the coin for the history request  
 -h, --help                         Print help
```

**Example:**

```sh
komodefi-cli wallet tx-history DOC --limit 5 --page 3     
Getting tx history, coin: DOC  
limit: 5  
skipped: 10  
total: 19  
page_number: 3  
total_pages: 4  
current_block: 208242  
sync_status: NotEnabled  
transactions:    
│ time: 23-07-24 16:54:29                                                                                                                              │  
│ coin: DOC                                                                                                                                            │  
│ block: 132256                                                                                                                                        │  
│ confirmations: 75987                                                                                                                                 │  
│ transaction_type: StandardTransfer                                                                                                                   │  
│ total_amount: 949836.47                                                                                                                              │  
│ spent_by_me: 949836.47                                                                                                                               │  
│ received_by_me: 949835.47                                                                                                                            │  
│ my_balance_change: -1.00                                                                                                                             │  
│ fee_details: {"type":"Utxo","coin":"DOC","amount":"0.00001"}                                                                                         │  
│ tx_hash: 671aaee83b6d870f168c4e0be93e2d2087c8eae324e105c1dcad240cfea73c03                                                                            │  
│ from: RPFGrvJWjSYN4qYvcXsECW1HoHbvQjowZM                                                                                                             │  
│ to: RPFGrvJWjSYN4qYvcXsECW1HoHbvQjowZM, RQvvz29iHpn4iuHeN7qFXnLbc1eh31nRKY                                                                           │  
│ internal_id: 671aaee83b6d870f168c4e0be93e2d2087c8eae324e105c1dcad240cfea73c03                                                                        │  
│ tx_hex: 0400008085202f8901afdad3b683400c1ff4331514afdcbd1703647edcd1abaa3d93b1aecd6daaa3e0010000006a473044022022d692771b413f4b197c2dba61453b97a4df99 │  
│ 4986fafbd8e8950fcc77e1519d022012c990b48e11ee568907184236e1d5e2f03a5a18132fb322fe7522e33ddc6f39012102264fcd9401d797c50fe2f1c7d5fe09bbc10f3838c1d8d6f7 │  
│ 93061fa5f38b2b4dffffffff0200e1f505000000001976a914abad17c49173537a07e39d2c94dcfe64645b1ad488ac54729d14635600001976a9149934ebeaa56cb597c936a9ed8202d8 │  
│ d97a0a700388acb0acbe64000000000000000000000000000000                                                                                                 │  
│                                                                                                                                                      │  
├──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────┤  
│ time: 23-07-24 16:44:09                                                                                                                              │  
│ coin: DOC                                                                                                                                            │
...
```

### wallet show-private-key (private)

The `show-private-key` command allows to get private key for the given coin that could be used to access account in a third-party wallet application. This command is proceed using the [`show_priv_key` RPC API method](https://developers.komodoplatform.com/basic-docs/atomicdex-api-legacy/show_priv_key.html).  *Aliases: `private`, `private-key`*

```sh
komodefi-cli wallet private --help  
Returns the private key of the specified coin in a format compatible with coin wallets  
  
Usage: komodefi-cli wallet show-priv-key <COIN>  
  
Arguments:  
 <COIN>  The name of the coin of the private key to show  
  
Options:  
 -h, --help  Print help
```

**Example:**

```sh
komodefi-cli wallet private-key DOC  
Getting private key, coin: DOC  
coin: DOC  
priv_key: UufMCuXDsLNWga8kz7ZyJKXzH86MZ7vbEzUa21A7EEYqqbtHwYQ5
```

### wallet validate-address (validate)

The `validate-address` validated wether the given coin address could exist and valid using the [`validateaddress` RPC API method](https://developers.komodoplatform.com/basic-docs/atomicdex-api-legacy/validateaddress.html). *Alias: `validate`*

```sh
release/komodefi-cli wallet validate-address --help  
Checks if an input string is a valid address of the specified coin  
  
Usage: komodefi-cli wallet validate-address <COIN> <ADDRESS>  
  
Arguments:  
 <COIN>     The coin to validate address for  
 <ADDRESS>  The input string to validate  
  
Options:  
 -h, --help  Print help
```

**Example:**

```sh
komodefi-cli wallet validate-address ZOMBIE zs1lgyuf2tudj7vgtx2pgtsqqdlcrva7a5rw4n4f7rswc6vnk8thc85dl4uvazpkv6y8yd25cjp72g  
Validating address, coin: ZOMBIE, address: zs1lgyuf2tudj7vgtx2pgtsqqdlcrva7a5rw4n4f7rswc6vnk8thc85dl4uvazpkv6y8yd25cjp72g  
valid: valid
```

### wallet kmd-rewards-info (rewards)

The `kmd-rewards-info` returns information about the active user rewards that can be claimed by an address's unspent outputs using the [`kmd_rewards_info` RPC API methods](https://developers.komodoplatform.com/basic-docs/atomicdex-api-legacy/kmd_rewards_info.html). *Alias: `rewards`*

```sh
komodefi-cli wallet rewards --help  
Informs about the active user rewards that can be claimed by an address's unspent outputs  
  
Usage: komodefi-cli wallet kmd-rewards-info  
  
Options:  
 -h, --help  Print help
```

**Example:**

```sh
komodefi-cli wallet rewards    
Getting kmd rewards info  
rewards_info:    
tx_hash: 498e23e1e0a3322192c4c4d7f531a562c12f44f86544013344397c3fc217963a  
height: 3515481  
output_index: 1  
amount: 1599.00  
locktime: 23-07-23 14:30:46  
accrued_rewards: 23-07-23 14:30:46  
accrued_rewards: 0.01356243  
accrue_start_at: 23-07-23 15:30:46  
accrue_stop_at: 23-08-23 14:30:46
```

### wallet convert-address (convert)

The `convert-address` command converts an input address to a specified address format using the [`convert_address` RPC API method](https://developers.komodoplatform.com/basic-docs/atomicdex-api-legacy/convertaddress.html). For example, this method can be used to convert a BCH address from legacy to cash address format and vice versa. Or this can be used to convert an ETH address from single to mixed case checksum format. *Alias: `convert`*

```sh
komodefi-cli wallet convert --help  
Converts an input address to a specified address format  
  
Usage: komodefi-cli wallet convert-address [OPTIONS] --coin <COIN> --from <FROM> --format <FORMAT>  
  
Options:  
 -c, --coin <COIN>  
         The name of the coin address context  
 -f, --from <FROM>  
         Input address  
 -F, --format <FORMAT>  
         Address format to which the input address should be converted [possible values: mixed-case, cash-address, standard]  
 -C, --cash-address-network <CASH_ADDRESS_NETWORK>  
         Network prefix for cashaddress format [possible values: bitcoin-cash, bch-test, bch-reg]  
 -h, --help  
         Print help
```

**Examples:**

Mixed case

```sh
komodefi-cli wallet convert --coin ETH --from 0xfb6916095ca1df60bb79ce92ce3ea74c37c5d359 --format mixed-case  
Converting address  
address: 0xfB6916095ca1df60bB79Ce92cE3Ea74c37c5d359
```

Standard

```sh
komodefi-cli wallet convert --coin BCH --from bitcoincash:qzxqqt9lh4feptf0mplnk58gnajfepzwcq9f2rxk55 --format standard  
Converting address  
address: 1DmFp16U73RrVZtYUbo2Ectt8mAnYScpqM
```

Cash address

```sh
komodefi-cli wallet convert --coin BCH --from 1DmFp16U73RrVZtYUbo2Ectt8mAnYScpqM --format cash-address -C bch-test  
Converting address  
address: bchtest:qzxqqt9lh4feptf0mplnk58gnajfepzwcqpmwyypng
```

### wallet convert-utxo-address (convert-utxo)

The `convert-utxo` command is similar to convert-address but is aimed to convert UTXO addresses to each other

```sh
komodefi-cli wallet convert-utxo-address <ADDRESS> <TO_COIN>  
  
Arguments:  
 <ADDRESS>  Input UTXO address  
 <TO_COIN>  Input address to convert from  
  
Options:  
 -h, --help  Print help
```

**Example:**

```sh
komodefi-cli wallet convert-utxo RPFGrvJWjSYN4qYvcXsECW1HoHbvQjowZM KMD  
Convert utxo address  
address: RPFGrvJWjSYN4qYvcXsECW1HoHbvQjowZM
```

### wallet get-public-key (public)

The `get-public-key` command returns the compressed secp256k1 pubkey corresponding to the configured seed phrase using the [`get_public_key` RPC API method](https://developers.komodoplatform.com/basic-docs/atomicdex-api-20/get_public_key.html). *Alias: `public`, `get-public`, `public-key`*

```sh
komodefi-cli wallet public --help  
Returns the compressed secp256k1 pubkey corresponding to the user's seed phrase  
  
Usage: komodefi-cli wallet get-public-key  
  
Options:  
 -h, --help  Print help
```

**Example:**

```sh
komodefi-cli wallet public  
Getting public key  
public_key: 02264fcd9401d797c50fe2f1c7d5fe09bbc10f3838c1d8d6f793061fa5f38b2b4d
```

### wallet get-public-key-hash (hash)


The `get-public-key-hash` command returns [RIPEMD-160](https://en.bitcoin.it/wiki/RIPEMD-160)(https://en.bitcoin.it/wiki/RIPEMD-160) hash version of the configured seed phrase using the [` get_public_key_hash` RPC API method](https://developers.komodoplatform.com/basic-docs/atomicdex-api-20/get_public_key_hash.html). *Aliases: `pubkey-hash`, `hash`, `pubhash`*.

```sh
komodefi-cli wallet pubhash --help  
Returns the RIPEMD-160 hash version of your public key  
  
Usage: komodefi-cli wallet get-public-key-hash  
  
Options:  
 -h, --help  Print help
```

**Example:**

```sh
komodefi-cli wallet pubhash    
Getting public key hash  
public_key_hash: 9934ebeaa56cb597c936a9ed8202d8d97a0a7003
```

## DEX funtionality

The next group of commands provides trading functionality, orders managing and trading commands are meant. General commands such as `sell`, `buy`, `set-price`, `update-maker-order` are left at the top level to make it easier to call them.


```sh
komodefi-cli orders --help  
Order listing commands: book, history, depth etc.  
  
Usage: komodefi-cli order <COMMAND>  
  
Commands:  
 orderbook        Get orderbook [aliases: book]  
 orderbook-depth  Get orderbook depth [aliases: depth]  
 order-status     Return the data of the order with the selected uuid created by the current node [aliases: status]  
 best-orders      Return the best priced trades available on the orderbook [aliases: best]  
 my-orders        Get my orders [aliases: my, mine]  
 orders-history   Return all orders whether active or inactive that match the selected filters [aliases: history, filter]  
 help             Print this message or the help of the given subcommand(s)  
  
Options:  
 -h, --help  Print help
```
### orders orderbook (book)

The `orderbook` command gets available bids and asks for the given base and rel coins using the [`orderbook` RPC API method](https://developers.komodoplatform.com/basic-docs/atomicdex-api-legacy/orderbook.html). You can specify which order should be shown and limit the output to the given ask and bids limits. *Alias: `book`*

```sh
komodefi-cli orders orderbook DOC MARTY --help  
Get orderbook  
  
Usage: komodefi-cli order orderbook [OPTIONS] <BASE> <REL>  
  
Arguments:  
 <BASE>  Base currency of a pair  
 <REL>   Related currency, also can be called "quote currency" according to exchange terms  
  
Options:  
 -u, --uuids                    Enable `uuid` column  
 -m, --min-volume               Enable `min_volume` column [aliases: min]  
 -M, --max-volume               Enable `max_volume` column [aliases: max]  
 -p, --publics                  Enable `public` column  
 -a, --address                  Enable `address` column  
 -A, --age                      Enable `age` column  
 -c, --conf-settings            Enable order confirmation settings column  
     --asks-limit <ASKS_LIMIT>  Orderbook asks count limitation [default: 20]  
     --bids-limit <BIDS_LIMIT>  Orderbook bids count limitation [default: 20]  
 -h, --help                     Print help
```


**Example:**

```sh
komodefi-cli orders book DOC MARTY --uuids --asks-limit 5 --bids-limit 5  
Getting orderbook, base: DOC, rel: MARTY  
     Volume: DOC Price: MARTY     Uuid                                    
*           1.00 3.00000000       4d7c187b-d61f-4349-b608-a6abe0b3f0ea    
            0.14 1.00000000       0e549623-fead-4645-9c6c-00877b50bac2    
     94974264.87 1.00000000       9406010e-54fd-404c-b252-1af473bf77e6    
- --------------- ---------------- ------------------------------------    
     94898462.11 1.00000000       5a14a592-feea-4eca-92d8-af79c0670a39    
            1.94 1.00000000       fbd51c38-f3a7-42c5-aa3a-c52938188086
```

### sell

The `sell` command sells a base coin for a rel one in a given volume and at a given price in rel using the [`sell` RPC API method](https://developers.komodoplatform.com/basic-docs/atomicdex-api-legacy/sell.html). That is possible to limit  a new order with the `--min-volume`.  If `--uuid` or `--public` options were set matching procedure would be restricted according to the given values. `--base-confs`, `--base-nota`, `--rel-confs`, `--rel-nota` are to override preset confirmation and notarization rules.

```sh
komodefi-cli sell  --help  
Put a selling request  
  
Usage: komodefi-cli sell [OPTIONS] <BASE> <REL> <VOLUME> <PRICE>  
  
Arguments:  
 <BASE>    Base currency of a pair  
 <REL>     Related currency of a pair  
 <VOLUME>  Amount of coins the user is willing to sell/buy of the base coin  
 <PRICE>   Price in rel the user is willing to receive/pay per one unit of the base coin  
  
Options:  
 -t, --order-type <ORDER_TYPE>  The GoodTillCancelled order is automatically converted to a maker order if not matched in 30 seconds, and this maker order stays in the orderbook until explicitly cancelled. On the other hand, a FillOrKi  
ll is cancelled if not matched within 30 seconds [default: good-till-cancelled] [aliases: type] [possible values: fill-or-kill, good-till-cancelled]  
 -m, --min-volume <MIN_VOLUME>  Amount of base coin that will be used as min_volume of GoodTillCancelled order after conversion to maker  
 -u, --uuid <MATCH_UUIDS>       The created order is matched using a set of uuid  
 -p, --public <MATCH_PUBLICS>   The created order is matched using a set of publics to select specific nodes  
     --base-confs <BASE_CONFS>  Number of required blockchain confirmations for base coin atomic swap transaction [aliases: bc]  
     --base-nota <BASE_NOTA>    Whether dPoW notarization is required for base coin atomic swap transaction [aliases: bn] [possible values: true, false]  
     --rel-confs <REL_CONFS>    Number of required blockchain confirmations for rel coin atomic swap transaction [aliases: rc]  
     --rel-nota <REL_NOTA>      Whether dPoW notarization is required for rel coin atomic swap transaction [aliases: rn] [possible values: true, false]  
 -s, --save-in-history          If true, each order's short record history is stored else the only order status will be temporarily stored while in progress [aliases: save]  
 -h, --help                     Print help
```

**Example**:

```sh
komodefi-cli sell DOC MARTY 1 3  
Selling: 1 DOC for: 3 MARTY at the price of 3 MARTY per DOC  
4d7c187b-d61f-4349-b608-a6abe0b3f0ea
```
### set-price

The `set-price` is almost the same as `sell` command, it's always considered as `sell`. When the `set-price` is called the maker order is created immidiatelly. It provides the `--cancel-prev` option alowing to cancel existing orders for the given pair. This command is implemented by requesting the [`setprice` RPC API method](https://developers.komodoplatform.com/basic-docs/atomicdex-api-legacy/setprice.html).

```sh
komodefi-cli set-price --help  
Place an order on the orderbook. The setprice order is always considered a sell  
  
Usage: komodefi-cli set-price [OPTIONS] <--max|--volume <VOLUME>> <BASE> <REL> <PRICE>  
  
Arguments:  
 <BASE>   The name of the coin the user desires to sell  
 <REL>    The name of the coin the user desires to receive  
 <PRICE>  The price in rel the user is willing to receive per one unit of the base coin  
  
Options:  
 -M, --max                      Use the entire coin balance for the order, taking 0.001 coins into reserve to account for fees  
 -v, --volume <VOLUME>          The maximum amount of base coin available for the order, ignored if max is true; the following values must be greater than or equal to the min_trading_vol of the corresponding coin  
 -m, --min-volume <MIN_VOLUME>  The minimum amount of base coin available for the order; it must be less or equal than volume param; the following values must be greater than or equal to the min_trading_vol of the corresponding coin  
 -c, --cancel-prev              Cancel all existing orders for the selected pair [aliases: cancel]  
     --base-confs <BASE_CONFS>  Number of required blockchain confirmations for base coin atomic swap transaction [aliases: bc]  
     --base-nota <BASE_NOTA>    Whether dPoW notarization is required for base coin atomic swap transaction [aliases: bn] [possible values: true, false]  
     --rel-confs <REL_CONFS>    Number of required blockchain confirmations for rel coin atomic swap transaction [aliases: rc]  
     --rel-nota <REL_NOTA>      Whether dPoW notarization is required for rel coin atomic swap transaction [aliases: rn] [possible values: true, false]  
 -s, --save-in-history          If true, each order's short record history is stored in a local SQLite database table, and when the order is cancelled or fully matched, it's history will be saved as a json file [aliases: save]  
 -h, --help                     Print help
```

**Example:**

```sh
komodefi-cli set-price DOC MARTY 3 --volume 1 -c  
Setting price for pair: DOC MARTY  
Maker order:    
               base: DOC  
                rel: MARTY  
              price: 3.00  
               uuid: 077b68d9-0e71-4a35-9b3f-c3cfe5b57310  
         created at: 23-09-15 13:40:28  
         updated at: 23-09-15 13:40:28  
       max_base_vol: 1.00  
       min_base_vol: 0.000100  
              swaps: empty  
      conf_settings: 1,false:1,false

komodefi-cli orders book DOC MARTY --uuids
Getting orderbook, base: DOC, rel: MARTY  
     Volume: DOC Price: MARTY     Uuid                                    
*           1.00 3.00000000       077b68d9-0e71-4a35-9b3f-c3cfe5b57310    
            0.14 1.00000000       0e549623-fead-4645-9c6c-00877b50bac2    
     94974264.87 1.00000000       a29748d0-fa6a-4a7f-a402-c569c96ea92a    
- -------------- ---------------- ------------------------------------    
     94898462.11 1.00000000       41b0a9cb-8416-4748-af4d-2064dd8ae617    
            1.94 1.00000000       fbd51c38-f3a7-42c5-aa3a-c52938188086
...
```

### update-maker-order (update)

The `update-maker-order` updates existing order selected by its uuid with the given values using the [`update_maker_order` RPC API method](https://developers.komodoplatform.com/basic-docs/atomicdex-api-legacy/update_maker_order.html).  The order should be changed according to the given settings like it was done in the former commands. Volume is changed by the delta that is set with `--volume-delta` or is set as max. *Alias: `update`*

```sh
komodefi-cli update-maker-order --uuid f398ffe2-9c74-4340-8702-68eca1d167e8 --help  
Update order on the orderbook  
  
Usage: komodefi-cli update-maker-order [OPTIONS] --uuid <UUID>  
  
Options:  
 -u, --uuid <UUID>                  Uuid of the order the user desires to update  
 -p, --price <PRICE>                Price in rel the user is willing to receive per one unit of the base coin  
 -M, --max-volume                   Whether to use the entire coin balance for the order, taking 0.001 coins into reserve to account for fees  
 -d, --volume-delta <VOLUME_DELTA>  Volume added to or subtracted from the max_base_vol of the order to be updated, resulting in the new volume which is the maximum amount of base coin available for the order, ignored if max is true  
 -m, --min-volume <MIN_VOLUME>      Minimum amount of base coin available for the order; it must be less or equal than the new volume; the following values must be greater than or equal to the min_trading_vol of the corresponding coin  
     --base-confs <BASE_CONFS>      Number of required blockchain confirmations for base coin atomic swap transaction [aliases: bc]  
     --base-nota <BASE_NOTA>        Whether dPoW notarization is required for base coin atomic swap transaction [aliases: bn] [possible values: true, false]  
     --rel-confs <REL_CONFS>        Number of required blockchain confirmations for rel coin atomic swap transaction [aliases: rc]  
     --rel-nota <REL_NOTA>          Whether dPoW notarization is required for rel coin atomic swap transaction [aliases: rn] [possible values: true, false]  
 -h, --help                         Print help
```

**Example:**

```sh
komodefi-cli update-maker-order --uuid 077b68d9-0e71-4a35-9b3f-c3cfe5b57310 -p 10 -d=10 --base-nota false --bc 2 --rn false --rc 2 -m 4    
Updating maker order  
Maker order:    
               base: DOC  
                rel: MARTY  
              price: 10.00  
               uuid: 077b68d9-0e71-4a35-9b3f-c3cfe5b57310  
         created at: 23-09-15 13:40:28  
         updated at: 23-09-15 13:43:31  
       max_base_vol: 21.00  
       min_base_vol: 4.00  
              swaps: empty  
      conf_settings: 2,false:2,false
      
komodefi-cli orders book DOC MARTY --uuids -c  
Getting orderbook, base: DOC, rel: MARTY  
     Volume: DOC Price: MARTY     Uuid                                 Order conf (bc,bn:rc,rn)    
*           21.00 10.00000000      077b68d9-0e71-4a35-9b3f-c3cfe5b57310 1,false:1,false             
            0.14 1.00000000       0e549623-fead-4645-9c6c-00877b50bac2 1,false:1,false             
     94974264.87 1.00000000       c4a469c7-fa69-4002-a1be-2578d329d681 1,false:1,false             
- --------------- ---------------- ------------------------------------ ------------------------    
     94898462.11 1.00000000       9734d9e5-36f5-4f9f-81b9-deb79390f82b 1,false:1,false             
            1.94 1.00000000       fbd51c38-f3a7-42c5-aa3a-c52938188086 1,false:1,false
...
```

*Notice: you can see the responded `conf-settings` for the given order 077b68d9-0e71-4a35-9b3f-c3cfe5b57310 are wrong*
### buy

The `buy` command buys a base coin for a rel one in a given volume and at a given price using the [`buy` RPC API method](https://developers.komodoplatform.com/basic-docs/atomicdex-api-legacy/buy.html). Like for the `sell` command that is possible to limit  a new order with the `--min-volume`.  If `--uuid` or `--public` options were set matching procedure would be restricted according to the given values. `--base-confs`, `--base-nota`, `--rel-confs`, `--rel-nota` are to override preset confirmation and notarization rules.

```sh
komodefi-cli buy --help  
Put a buying request  
  
Usage: komodefi-cli buy [OPTIONS] <BASE> <REL> <VOLUME> <PRICE>  
  
Arguments:  
 <BASE>    Base currency of a pair  
 <REL>     Related currency of a pair  
 <VOLUME>  Amount of coins the user is willing to sell/buy of the base coin  
 <PRICE>   Price in rel the user is willing to receive/pay per one unit of the base coin  
  
Options:  
 -t, --order-type <ORDER_TYPE>  The GoodTillCancelled order is automatically converted to a maker order if not matched in 30 seconds, and this maker order stays in the orderbook until explicitly cancelled. On the other hand, a FillOrKi  
ll is cancelled if not matched within 30 seconds [default: good-till-cancelled] [aliases: type] [possible values: fill-or-kill, good-till-cancelled]  
 -m, --min-volume <MIN_VOLUME>  Amount of base coin that will be used as min_volume of GoodTillCancelled order after conversion to maker  
 -u, --uuid <MATCH_UUIDS>       The created order is matched using a set of uuid  
 -p, --public <MATCH_PUBLICS>   The created order is matched using a set of publics to select specific nodes  
     --base-confs <BASE_CONFS>  Number of required blockchain confirmations for base coin atomic swap transaction [aliases: bc]  
     --base-nota <BASE_NOTA>    Whether dPoW notarization is required for base coin atomic swap transaction [aliases: bn] [possible values: true, false]  
     --rel-confs <REL_CONFS>    Number of required blockchain confirmations for rel coin atomic swap transaction [aliases: rc]  
     --rel-nota <REL_NOTA>      Whether dPoW notarization is required for rel coin atomic swap transaction [aliases: rn] [possible values: true, false]  
 -s, --save-in-history          If true, each order's short record history is stored else the only order status will be temporarily stored while in progress [aliases: save]  
 -h, --help                     Print help
```

**Example**

```sh
komodefi-cli buy DOC MARTY 1 0.1    
Buying: 1 DOC with: 0.1 MARTY at the price of 0.1 MARTY per DOC
2e791413-65ba-4a2a-a010-e1ef521f1f73

komodefi-cli orders book DOC MARTY --uuids --asks-limit 5 --bids-limit 5  
Getting orderbook, base: DOC, rel: MARTY  
     Volume: DOC Price: MARTY     Uuid                                    
...
- --------------- ---------------- ------------------------------------    
...
*           1.00 0.10000000       2e791413-65ba-4a2a-a010-e1ef521f1f73
```

### order order-status (status)

The `order-status` command gets the status and details of an order by the given uuid using the [`order_status` RPC API method](https://developers.komodoplatform.com/basic-docs/atomicdex-api-legacy/order_status.html). *Alias: `status`*

```sh
komodefi-cli order status --help  
Return the data of the order with the selected uuid created by the current node  
  
Usage: komodefi-cli order order-status <UUID>  
  
Arguments:  
 <UUID>     
  
Options:  
 -h, --help  Print help
```

**Example:**

```sh
komodefi-cli order status 077b68d9-0e71-4a35-9b3f-c3cfe5b57310  
Getting order status: 077b68d9-0e71-4a35-9b3f-c3cfe5b57310  
               base: DOC  
                rel: MARTY  
              price: 10.00  
               uuid: 077b68d9-0e71-4a35-9b3f-c3cfe5b57310  
         created at: 23-09-15 13:40:28  
         updated at: 23-09-15 13:43:31  
       max_base_vol: 21.00  
       min_base_vol: 4.00  
              swaps: empty  
      conf_settings: 2,false:2,false  
        cancellable: true  
   available_amount: 21.00
```

### orders orderbook-depth (depth)

The `orderbook-depth` command gets a common orderbooks' view on the given set of pairs using the [`orderbook_depth` RPC API method](https://developers.komodoplatform.com/basic-docs/atomicdex-api-legacy/orderbook_depth.html). Each resulting value is a count of bids and asks in the related orderbook. *Alias: `depth`*

```sh
komodefi-cli orders depth --help  
Get orderbook depth  
  
Usage: komodefi-cli order orderbook-depth <BASE/REL>...  
  
Arguments:  
 <BASE/REL>...     
  
Options:  
 -h, --help  Print help
```

**Example:**

```sh
komodefi-cli orders depth DOC/MARTY BTC/KMD ETH/BTC  
Getting orderbook depth for pairs: DOC/MARTY, BTC/KMD, ETH/BTC  
           Bids Asks    
DOC/MARTY: 3    3       
  BTC/KMD: 8    5       
  ETH/BTC: 2    0
```
### orders best-orders (best)

The `best-orders` command scans all order books related to a given coin and lists the more profitable orders available in them that can meet a given `--volume` or limits them with a given `--number`.  This command is implemented by requesting the [`best_orders` RPC API method](https://developers.komodoplatform.com/basic-docs/atomicdex-api-legacy/best_orders.html).

```sh
komodefi-cli orders best-orders --help  
Return the best priced trades available on the orderbook  
  
Usage: komodefi-cli order best-orders [OPTIONS] <--volume <VOLUME>|--number <NUMBER>> <COIN> <ACTION>  
  
Arguments:  
 <COIN>    The coin to get best orders  
 <ACTION>  Whether to buy or sell the selected coin [possible values: buy, sell]  
  
Options:  
 -v, --volume <VOLUME>    The returned results will show the best prices for trades that can fill the requested volume  
 -n, --number <NUMBER>    The returned results will show a list of the best prices  
 -o, --show-orig-tickets  Whether to show the original tickers if they are configured for the queried coin [aliases: show-origin, original-tickers, origin]  
 -e, --exclude-mine       Exclude orders that is mine  
 -h, --help               Print help
```

**Example:**

```sh
komodefi-cli orders best-orders --volume 1 BTC sell  
Getting best orders: Sell BTC  
│  │ Price       │ Uuid                                 │ Base vol(min:max) │ Rel vol(min:max)  │ Address                                                │ Confirmation     │  
│ ARRR                                                                                                                                                                      │  
│  │ 129058.90   │ 2157dcfe-a352-4f72-b4b2-327b3021a013 │ 0.0080:0.0085     │ 1034.68:1104.35   │ Shielded                                               │ 1,false:2,true   │  
│  │ 144073.53   │ a56072bf-8a69-423b-a351-ed582cd43209 │ 0.0081:0.14       │ 1175.57:20787.01  │ Shielded                                               │ 1,false:2,true   │  
│ BAND-BEP20                                                                                                                                                                │  
│  │ 25589.58    │ e88c6f78-46ea-462e-ad0d-ebd9b49323e7 │ 0.0081:0.01       │ 208.53:493.22     │ 0xadb681c3a1ec9bbc4105b8e8eb5fc7178125b450             │ 1,false:3,false  │  
│ BCH                                                                                                                                                                       │  
│  │ 118.06      │ 1dda5e08-e73a-4f8a-a37e-6201e84dbf74 │ 0.0081:0.0099     │ 0.96:1.17         │ bitcoincash:qp42lwm4xvgg2sjhy7nc49qwjv60dqdnu5u2h2zaya │ 1,false:1,false  │
...
```

### orders my-orders (my, mine)

The `my-orders` command requests information about orders created by the current node (running `mm2` instance) using the [`my_orders` RPC API method](https://developers.komodoplatform.com/basic-docs/atomicdex-api-legacy/my_orders.html). *Aliases: `my`, `mine`.

**Example:**

```sh
komodefi-cli orders mine  
Getting my orders  
       Taker orders: empty  
  
       Maker orders:    
│ base,rel   │ price  │ uuid                                 │ created at,        │ min base vol, │ cancellable │ available │ swaps │ conf_settings     │ history changes │  
│            │        │                                      │ updated at         │ max base vol  │             │ amount    │       │                   │                 │  
│ RICK,MORTY │ 2.00   │ 1fc0df20-9c21-461a-ad78-a4b37d4ab336 │ 23-05-08 12:07:18, │ 0.000100,     │ true        │ 0.50      │ empty │ 555,true:111,true │ none            │  
│            │        │                                      │ 23-05-08 12:07:18  │ 0.50          │             │           │       │                   │                 │  
│ DOC,KMD    │ 100.00 │ 009b9b1c-4582-4ec7-aef9-2d6729e8cde2 │ 23-09-15 15:35:48, │ 0.000100,     │ true        │ 1.00      │ empty │ 1,false:2,true    │ none            │  
│            │        │                                      │ 23-09-15 15:35:48  │ 1.00          │             │           │       │                   │                 │
```

### orders orders-history (history)

The `orders-history` command requests the hitstory of orders created by the current node (running `mm2` instance) using the [` orders_history_by_filter` RPC API method](https://developers.komodoplatform.com/basic-docs/atomicdex-api-legacy/orders_history_by_filter.html). Resulting orders are queried by the given filter.

```sh
komodefi-cli orders history --help  
Return all orders whether active or inactive that match the selected filters  
  
Usage: komodefi-cli order orders-history [OPTIONS] <--takers|--makers|--warnings|--all>  
  
Options:  
 -t, --takers                     Whether to show taker orders detailed history  
 -m, --makers                     Whether to show maker orders detailed history  
 -w, --warnings                   Whether to show warnings  
 -a, --all                        Whether to show common history data  
     --type <ORDER_TYPE>          Return only orders that match the type [possible values: taker, maker]  
     --action <INITIAL_ACTION>    Return only orders that match the initial action. Note that maker order initial_action is considered "Sell" [possible values: sell, buy]  
     --base <BASE>                Return only orders that match the order.base  
     --rel <REL>                  Return only orders that match the order.rel  
     --from-price <FROM_PRICE>    Return only orders whose price is more or equal the from_price  
     --to-price <TO_PRICE>        Return only orders whose price is less or equal the to_price  
     --from-volume <FROM_VOLUME>  Return only orders whose volume is more or equal the from_volume  
     --to-volume <TO_VOLUME>      Return only orders whose volume is less or equal the to_volume  
     --from-dt <FROM_DT>          Return only orders that match the order.created_at >= from_dt. Datetime fmt: "%y-%m-%dT%H:%M:%S"  
     --to-dt <TO_DT>              Return only orders that match the order.created_at <= to_dt. Datetime fmt: "%y-%m-%dT%H:%M:%S"  
     --was-taker                  Return only GoodTillCancelled orders that got converted from taker to maker  
     --status <STATUS>            Return only orders that match the status [possible values: created, updated, fulfilled, insuficcient-balance, cancelled, timed-out]  
 -h, --help                       Print help
```

**Example:**

Requesting all DOC based orders

```sh
komodefi-cli orders history --base DOC --all    
Getting order history  
Orders history:  
│uuid                                │Type │Action│Base│Rel  │Volume│Price │Status   │Created          │Updated          │Was taker│  
│                                    │     │      │    │     │      │      │         │                 │                 │         │  
│22532835-8d93-4484-bb6b-e01be0acbde0│Maker│Sell  │DOC │MARTY│1.00  │3.00  │Cancelled│23-09-15 15:42:06│23-09-15 15:42:31│false    │  
│009b9b1c-4582-4ec7-aef9-2d6729e8cde2│Maker│Sell  │DOC │KMD  │1.00  │100.00│Created  │23-09-15 15:35:48│23-09-15 15:35:48│false    │  
│72ad0098-a684-4ac1-925b-9b7155faa22a│Maker│Sell  │DOC │MARTY│1.00  │3.00  │Cancelled│23-09-15 15:34:29│23-09-15 15:36:30│false    │  
│077b68d9-0e71-4a35-9b3f-c3cfe5b57310│Maker│Sell  │DOC │MARTY│21.00 │10.00 │Cancelled│23-09-15 13:40:28│23-09-15 15:18:18│false    │  
│0fc8996a-caec-4323-8218-93882c317f88│Maker│Sell  │DOC │MARTY│1.00  │3.00  │Cancelled│23-09-15 13:39:38│23-09-15 13:40:28│false    │  
│26a51d94-6957-4c76-b6e6-d31f3cf5e4a6│Maker│Sell  │DOC │MARTY│1.00  │0.90  │Cancelled│23-09-15 08:57:45│23-09-15 13:40:28│false    │  
│f398ffe2-9c74-4340-8702-68eca1d167e8│Maker│Sell  │DOC │MARTY│9.00  │10.00 │Cancelled│23-09-14 13:59:27│23-09-15 08:57:45│false    │  
│96b193bf-ac34-48f4-8dc8-c59c82149753│Maker│Sell  │DOC │MARTY│1.00  │3.00  │Cancelled│23-09-14 13:45:17│23-09-14 13:59:27│false    │
```

Requesting detailed makers

```sh
komodefi-cli orders history --base RICK --makers     
Getting order history  
Maker orders history detailed:  
│ base,rel      │ price         │ uuid                                 │ created at,        │ min base vol, │ swaps                                 │ conf_settings   │ history changes │ orderbook ticker │                │  
│               │               │                                      │ updated at         │ max base vol  │                                       │                 │                 │ base, rel        │                │  
│               │               │                                      │                    │               │                                       │                 │                 │                  │                │  
│ RICK,MORTY    │ 0.50          │ e2db79f4-376e-4917-be37-f383c5516e28 │ 23-07-18 16:50:31, │ 0.000100,     │ empty                                 │ 10,true:10,true │ none            │ none             │                │  
│               │               │                                      │ 23-07-18 16:58:10  │ 0.10          │                                       │                 │                 │ none             │                │  
│ RICK,MORTY    │ 0.90          │ 1c739304-dd83-466e-8df4-ef99dc40afb9 │ 23-07-18 09:11:48, │ 0.00011,      │ empty                                 │ 10,true:10,true │ none            │ none             │                │  
│               │               │                                      │ 23-07-18 09:12:50  │ 0.10          │                                       │                 │                 │ none             │                │  
│ RICK,MORTY    │ 0.95          │ f1bf7c76-806e-40cb-a489-a52056ec42e6 │ 23-06-30 09:05:34, │ 0.00010,      │ fe6099a2-e29a-441b-be4f-21dd3666efad, │ 1,false:1,false │ none            │ none             │                │  
│               │               │                                      │ 23-06-30 09:05:34  │ 1.00          │ e07bcf02-788f-407c-a182-63c506280ca4, │                 │                 │ none             │                │  
│               │               │                                      │                    │               │ dec7fe39-16be-42cc-b3ba-2078ed5019b5, │                 │                 │                  │                │  
│               │               │                                      │                    │               │ c63bc058-72e7-4b1b-8866-e1e5d2a2253b, │                 │                 │                  │                │  
│               │               │                                      │                    │               │ 824fa32e-865f-49f1-b499-ba90b7141c2b  │                 │                 │                  │                │  
│ matches                                                                                                                                                                                                                   │  
│                       uuid: dec7fe39-16be-42cc-b3ba-2078ed5019b5                                                                                                                                                          │  
│                   req.uuid: dec7fe39-16be-42cc-b3ba-2078ed5019b5                                                                                                                                                          │  
│             req.(base,rel): MORTY(0.0100), RICK(0.01)                                                                                                                                                                     │  
│               req.match_by: Any                                                                                                                                                                                           │  
│                 req.action: Sell                                                                                                                                                                                          │  
│          req.conf_settings: 1,false:1,false                                                                                                                                                                               │  
│         req.(sender, dest): 144ee16a5960c50a930c26c0e01133de603eb41ce2e2e61e744fcfa76d4ffade,0000000000000000000000000000000000000000000000000000000000000000                                                             │  
│        reserved.(base,rel): RICK(0.01), MORTY(0.0100)                                                                                                                                                                     │  
│    reserved.(taker, maker): dec7fe39-16be-42cc-b3ba-2078ed5019b5,f1bf7c76-806e-40cb-a489-a52056ec42e6                                                                                                                     │  
│    reserved.(sender, dest): 264fcd9401d797c50fe2f1c7d5fe09bbc10f3838c1d8d6f793061fa5f38b2b4d,144ee16a5960c50a930c26c0e01133de603eb41ce2e2e61e744fcfa76d4ffade                                                             │  
│     reserved.conf_settings: 1,false:1,false                                                                                                                                                                               │  
│    connected.(taker,maker): dec7fe39-16be-42cc-b3ba-2078ed5019b5,f1bf7c76-806e-40cb-a489-a52056ec42e6                                                                                                                     │  
│   connected.(sender, dest): 264fcd9401d797c50fe2f1c7d5fe09bbc10f3838c1d8d6f793061fa5f38b2b4d,0000000000000000000000000000000000000000000000000000000000000000                                                             │  
│      connect.(taker,maker): dec7fe39-16be-42cc-b3ba-2078ed5019b5,f1bf7c76-806e-40cb-a489-a52056ec42e6                                                                                                                     │  
│     connect.(sender, dest): 0000000000000000000000000000000000000000000000000000000000000000,0000000000000000000000000000000000000000000000000000000000000000                                                             │  
│               last_updated: 23-06-30 09:54:13                                                                                                                                                                             │  
│                                                                                                                                                                                                                           │
...
```

## Swaps

Once an order is created and matched with another order, it is no longer part of the order book, the swap is created and execution begins

```sh
komodefi-cli swaps  
Swap related commands  
  
Usage: komodefi-cli swaps <COMMAND>  
  
Commands:  
 active-swaps, -a           Get all the swaps that are currently running [aliases: active]  
 my-swap-status, -s         Return the data of an atomic swap [aliases: status]  
 my-recent-swaps, -r        Return the data of the most recent atomic swaps by filter [aliases: recent]  
 recover-funds-of-swap, -R  Reclaim the user funds from the swap-payment address, if possible [aliases: recover, recover-funds, refund]  
 min-trading-vol            Return the minimum required volume for buy/sell/setprice methods for the selected coin  
 max-taker-vol              Return the maximum available volume for buy/sell methods for selected coin. The result should be used as is for sell method or divided by price for buy method.  
 trade-preimage             Return the approximate fee amounts that are paid per the whole swap [aliases: preimage]  
 help                       Print this message or the help of the given subcommand(s)  
  
Options:  
 -h, --help  Print help
```
###  swaps active-swaps (active)

The `active-swaps` command requests a list of swaps using the [`active_swaps` RPC API method](https://developers.komodoplatform.com/basic-docs/atomicdex-api-legacy/active_swaps.html). The output includes information about completed and active stages, hashes, and related transaction data. If detailed information was not requested using the `--include-status` option, only the uuids of active swaps would be listed.

```
komodefi-cli swap active --help  
Get all the swaps that are currently running  
  
Usage: komodefi-cli swaps {active-swaps|-a} [OPTIONS]  
  
Options:  
 -s, --include-status  Whether to include swap statuses in response; defaults to false  
 -h, --help            Print help
```

**Example:**
Getting only uuids:

```sh
komodefi-cli swap active    
Getting active swaps  
uuids:    
64fced3d-a0c2-423e-9fa5-f6c470448983
```

Getting detailed information
```sh
komodefi-cli swaps active -s  
Getting active swaps  
  
TakerSwap: 64fced3d-a0c2-423e-9fa5-f6c470448983  
my_order_uuid: 64fced3d-a0c2-423e-9fa5-f6c470448983  
gui: komodefi-cli  
mm_version: 1.0.7-beta_a611ca37f  
taker_coin: KMD  
taker_amount: 1.00  
taker_coin_usd_price: 0.22  
maker_coin: BCH  
maker_amount: 0.0010  
maker_coin_usd_price: 213.30  
events:    
│ Started                           │ uuid: 64fced3d-a0c2-423e-9fa5-f6c470448983                                                                             │  
│ 23-09-15 12:20:00                 │ started_at: 23-09-15 12:20:00                                                                                          │  
│                                   │ taker_coin: KMD                                                                                                        │  
│                                   │ maker_coin: BCH                                                                                                        │  
│                                   │ maker: 15d9c51c657ab1be4ae9d3ab6e76a619d3bccfe830d5363fa168424c0d044732                                                │  
│                                   │ my_persistent_pub: 02264fcd9401d797c50fe2f1c7d5fe09bbc10f3838c1d8d6f793061fa5f38b2b4d                                  │  
│                                   │ lock_duration: 31200                                                                                                   │  
│                                   │ maker_amount: 0.0010                                                                                                   │  
│                                   │ taker_amount: 1.00                                                                                                     │  
│                                   │ maker_payment_confirmations: 3                                                                                         │  
│                                   │ maker_payment_requires_nota: false                                                                                     │  
│                                   │ taker_payment_confirmations: 2                                                                                         │  
│                                   │ taker_payment_requires_nota: false                                                                                     │  
│                                   │ tacker_payment_lock: 23-09-15 21:00:00                                                                                 │  
│                                   │ maker_payment_wait: 23-09-15 15:48:00                                                                                  │  
│                                   │ maker_coin_start_block: 810794                                                                                         │  
│                                   │ taker_coin_start_block: 3590943                                                                                        │  
│                                   │ fee_to_send_taker_fee: coin: KMD, amount: 0.00001, paid_from_trading_vol: false                                        │  
│                                   │ taker_payment_trade_fee: coin: KMD, amount: 0.00001, paid_from_trading_vol: false                                      │  
│                                   │ maker_payment_spend_trade_fee: coin: BCH, amount: 0.00001, paid_from_trading_vol: true                                 │  
│                                   │ maker_coin_htlc_pubkey: 02264fcd9401d797c50fe2f1c7d5fe09bbc10f3838c1d8d6f793061fa5f38b2b4d                             │  
│                                   │ taker_coin_htlc_pubkey: 02264fcd9401d797c50fe2f1c7d5fe09bbc10f3838c1d8d6f793061fa5f38b2b4d                             │  
│                                   │                                                                                                                        │  
│ Negotiated                        │ maker_payment_locktime: 23-09-16 05:39:59                                                                              │  
│ 23-09-15 12:20:15                 │ maker_pubkey: 000000000000000000000000000000000000000000000000000000000000000000                                       │  
│                                   │ secret_hash: 2846d8eb4f442286158888a2231e577f0373b750                                                                  │  
│                                   │ maker_coin_htlc_pubkey: 0315d9c51c657ab1be4ae9d3ab6e76a619d3bccfe830d5363fa168424c0d044732                             │  
│                                   │ taker_coin_htlc_pubkey: 0315d9c51c657ab1be4ae9d3ab6e76a619d3bccfe830d5363fa168424c0d044732                             │  
│                                   │                                                                                                                        │  
│ TakerFeeSent                      │ tx_hex: 0400008085202f89013a9617c23f7c394433014465f8442fc162a531f5d7c4c4922132a3e0e1238e49010000006a4730440220552879ed │  
│ 23-09-15 12:20:15                 │ 2fae025920c0d3993a1e48955f5de1a999dcb43141ec569fca3ecaf602205f176776affed034b88bd3807300ff90d09e7d05edcb9a4b2189238dab │  
│                                   │ e0f436012102264fcd9401d797c50fe2f1c7d5fe09bbc10f3838c1d8d6f793061fa5f38b2b4dffffffff0276c40100000000001976a914ca1e0474 │  
│                                   │ 5e8ca0c60d8c5881531d51bec470743f88ac7082e33a250000001976a9149934ebeaa56cb597c936a9ed8202d8d97a0a700388ac744a0465000000 │  
│                                   │ 000000000000000000000000                                                                                               │  
│                                   │ tx_hash: 2d2b4745ed039a897bf78f45d83c939b508ff8f0436281d202eb1ccf402e2e37                                              │  
│                                   │                                                                                                                        │  
│ TakerPaymentInstructionsReceived  │ none                                                                                                                   │  
│ 23-09-15 12:20:16                 │                                                                                                                        │  
│                                   │                                                                                                                        │  
│ MakerPaymentReceived              │ tx_hex: 0100000001e91a267e855f865dbf741a34cc4509b233c62b9ec10d53afdea29238b9412812000000006a47304402202f82ea70d750dcc7 │  
│ 23-09-15 12:20:16                 │ eb003699fc052fcef4b216ef7904a5aedf90d88f1d7b0e6e02202e3b711fd950c48f5e6f8380aa0b47dd542ddd6970d997bbc21e510b2cf6d49941 │  
│                                   │ 210315d9c51c657ab1be4ae9d3ab6e76a619d3bccfe830d5363fa168424c0d044732ffffffff030e9201000000000017a9146a16cb1d0da650d083 │  
│                                   │ cf17d34fe2c01e2f4ede18870000000000000000166a142846d8eb4f442286158888a2231e577f0373b750c3530100000000001976a9141462c3dd │  
│                                   │ 3f936d595c9af55978003b27c250441f88ac004c0465                                                                           │  
│                                   │ tx_hash: 4cc4a5c89df668ac4aca619f10de74863b89374b57b18586ae113e7fe8c63e4a                                              │  
│                                   │                                                                                                                        │  
│ MakerPaymentWaitConfirmStarted    │                                                                                                                        │  
│ 23-09-15 12:20:16                 │                                                                                                                        │  
│                                   │                                                                                                                        │  
│ MakerPaymentValidatedAndConfirmed │                                                                                                                        │  
│ 23-09-15 12:59:34                 │                                                                                                                        │  
│                                   │                                                                                                                        │  
│ TakerPaymentSent                  │ tx_hex: 0400008085202f8901372e2e40cf1ceb02d2816243f0f88f509b933cd8458ff77b899a03ed45472b2d010000006b483045022100afc98d │  
│ 23-09-15 12:59:34                 │ 939ce43081eda416e8b5e63e2d36de3314f715e4555bf36c61d8654d9302207f8c33ef39be5cf93ffcb40e7ec64691f8205b5bf2b9caa1cf454051 │  
│                                   │ f69b89ad012102264fcd9401d797c50fe2f1c7d5fe09bbc10f3838c1d8d6f793061fa5f38b2b4dffffffff0300e1f5050000000017a9145a2ba5f8 │  
│                                   │ dd563ab9dcbb74a5fe6b5a683af94520870000000000000000166a142846d8eb4f442286158888a2231e577f0373b750889ded34250000001976a9 │  
│                                   │ 149934ebeaa56cb597c936a9ed8202d8d97a0a700388ac384d0465000000000000000000000000000000                                   │  
│                                   │ tx_hash: 90d73c9311ff72ca4bc79b844e3db6ec3a13c37c4996e399361b9711dad81e0a                                              │  
│                                   │                                                                                                                        │  
│ TakerPaymentSpent                 │ tx_hex: 0400008085202f89010a1ed8da11971b3699e396497cc3133aecb63d4e849bc74bca72ff11933cd79000000000d7473044022041e02ad7 │  
│ 23-09-15 13:02:16                 │ b050ef5a3fa90f7811b810c5c15b9fa52218768bed486131f44d0c450220037b2d5883dd3d56d4fa690a733aa1906e9156f926c5b6f6f9fda745be │  
│                                   │ 9ec4bb0120f849998cb9a3211fca8d6ba961b9aac27b4c1385597a62727e230ccfb086bae7004c6b6304d0c50465b1752102264fcd9401d797c50f │  
│                                   │ e2f1c7d5fe09bbc10f3838c1d8d6f793061fa5f38b2b4dac6782012088a9142846d8eb4f442286158888a2231e577f0373b75088210315d9c51c65 │  
│                                   │ 7ab1be4ae9d3ab6e76a619d3bccfe830d5363fa168424c0d044732ac68ffffffff0118ddf505000000001976a9141462c3dd3f936d595c9af55978 │  
│                                   │ 003b27c250441f88acd0c50465000000000000000000000000000000                                                               │  
│                                   │ tx_hash: 8c3db71bafdc3bc251b3b735e7b66c4eeec6e7081d751d7e1a35fe410d532f1a                                              │  
│                                   │ secret: f849998cb9a3211fca8d6ba961b9aac27b4c1385597a62727e230ccfb086bae7                                               │
```

### swaps my-swap-status (status)

The `my-swap-status` command requests the detailed information about completed and active stages, hashes and related transaction data using the [`my_swap_status` RPC API method](https://developers.komodoplatform.com/basic-docs/atomicdex-api-legacy/my_swap_status.html). The details provided by `my-swap-status` have the same format as the details provided by `active-swaps` command. *Alias: `status`*

**Example:**

```sh
komodefi-cli swap status 64fced3d-a0c2-423e-9fa5-f6c470448983  
Getting swap status: 64fced3d-a0c2-423e-9fa5-f6c470448983  
my_coin: KMD  
other_coin: BCH  
my_amount: 1.00  
other_amount: 0.0010  
started_at: 23-09-15 12:20:00  
recoverable: false  
TakerSwap: 64fced3d-a0c2-423e-9fa5-f6c470448983  
my_order_uuid: 64fced3d-a0c2-423e-9fa5-f6c470448983  
gui: komodefi-cli  
mm_version: 1.0.7-beta_a611ca37f  
taker_coin: KMD  
taker_amount: 1.00  
taker_coin_usd_price: 0.22  
maker_coin: BCH  
maker_amount: 0.0010  
maker_coin_usd_price: 213.30  
events:    
│ Started                           │ uuid: 64fced3d-a0c2-423e-9fa5-f6c470448983                                                                             │  
│ 23-09-15 12:20:00                 │ started_at: 23-09-15 12:20:00                                                                                          │  
│                                   │ taker_coin: KMD                                                                                                        │  
│                                   │ maker_coin: BCH                                                                                                        │  
│                                   │ maker: 15d9c51c657ab1be4ae9d3ab6e76a619d3bccfe830d5363fa168424c0d044732                                                │  
│                                   │ my_persistent_pub: 02264fcd9401d797c50fe2f1c7d5fe09bbc10f3838c1d8d6f793061fa5f38b2b4d                                  │  
│                                   │ lock_duration: 31200                                                                                                   │  
│                                   │ maker_amount: 0.0010                                                                                                   │  
│                                   │ taker_amount: 1.00                                                                                                     │  
│                                   │ maker_payment_confirmations: 3                                                                                         │  
│                                   │ maker_payment_requires_nota: false                                                                                     │  
│                                   │ taker_payment_confirmations: 2                                                                                         │  
│                                   │ taker_payment_requires_nota: false                                                                                     │  
│                                   │ tacker_payment_lock: 23-09-15 21:00:00                                                                                 │  
│                                   │ maker_payment_wait: 23-09-15 15:48:00                                                                                  │  
│                                   │ maker_coin_start_block: 810794                                                                                         │  
│                                   │ taker_coin_start_block: 3590943                                                                                        │  
│                                   │ fee_to_send_taker_fee: coin: KMD, amount: 0.00001, paid_from_trading_vol: false                                        │  
│                                   │ taker_payment_trade_fee: coin: KMD, amount: 0.00001, paid_from_trading_vol: false                                      │  
│                                   │ maker_payment_spend_trade_fee: coin: BCH, amount: 0.00001, paid_from_trading_vol: true                                 │  
│                                   │ maker_coin_htlc_pubkey: 02264fcd9401d797c50fe2f1c7d5fe09bbc10f3838c1d8d6f793061fa5f38b2b4d                             │  
│                                   │ taker_coin_htlc_pubkey: 02264fcd9401d797c50fe2f1c7d5fe09bbc10f3838c1d8d6f793061fa5f38b2b4d                             │  
│                                   │                                                                                                                        │
...
```

### swaps my-recent-swaps (recent)

The `my-recent-swaps` command requests the swap history selecte by the query of the following options: `--from-uuid`, `--my-coin`, `--other-coin`, `--from-timestamp`, `--to-timestamp`. The requesting swaps are organized into pages, there is the `--page-number` (`--page`) options that allows to select the certain one.

```sh
komodefi-cli swaps  recent --help  
Return the data of the most recent atomic swaps by filter  
  
Usage: komodefi-cli swaps {my-recent-swaps|-r} [OPTIONS]  
  
Options:  
 -l, --limit <LIMIT>  
         Limits the number of returned swaps [default: 10]  
 -u, --from-uuid <FROM_UUID>  
         Skip records until this uuid, skipping the from_uuid as well  
 -p, --page-number <PAGE_NUMBER>  
         Return limit swaps from the selected page; This param will be ignored if from_uuid is set  
 -m, --my-coin <MY_COIN>  
         Return only swaps that match the swap.my_coin = request.my_coin condition [aliases: mine]  
 -o, --other-coin <OTHER_COIN>  
         Return only swaps that match the swap.other_coin = request.other_coin condition [aliases: other]  
 -t, --from-timestamp <FROM_TIMESTAMP>  
         Return only swaps that match the swap.started_at >= request.from_timestamp condition. Datetime fmt: "%y-%m-%dT%H:%M:%S"  
 -T, --to-timestamp <TO_TIMESTAMP>  
         Return only swaps that match the swap.started_at < request.to_timestamp condition. Datetime fmt: "%y-%m-%dT%H:%M:%S"  
 -h, --help  
         Print help
```

**Example:**

Requesting forth page of swaps happened before 2023/07/02 12:21:01 by 2 swaps per page

```sh
komodefi-cli swaps  recent --to-timestamp 23-07-01T12:21:01 --limit 2 --page 4  
Getting recent swaps  
skipped: 6  
limit: 2  
total: 104  
page_number: 4  
total_pages: 52  
found_records: 2  
  
TakerSwap: f3237905-43e5-42a7-8285-98e0c122b625  
my_order_uuid: f3237905-43e5-42a7-8285-98e0c122b625
...
```

### swap recover-funds-of-swap (refund)

The `recover-funds-of-swap` requests the refund of funds that stuck on the blockchain due to the interrupted swap using the [` recover_funds_of_swap` RPC API method].  The given swap should be recoverable. *Aliases: `recover`, `recover-funds`, `refund`*

```sh
komodefi-cli swaps refund --help  
Reclaim the user funds from the swap-payment address, if possible  
  
Usage: komodefi-cli swaps {recover-funds-of-swap|-R} <UUID>  
  
Arguments:  
 <UUID>  Uuid of the swap to recover the funds  
  
Options:  
 -h, --help  Print help
```

### swaps min-trading-vol

The `min-trading-vol` command  provides the minimum required volume to trade the given coin using the [`min_trading_vol` RPC API method](https://developers.komodoplatform.com/basic-docs/atomicdex-api-legacy/min_trading_vol.html). This command allows the user to have a picture of what the trading volume might be for a given coin.

```sh
komodefi-cli swaps min-trading-vol --help  
Return the minimum required volume for buy/sell/setprice methods for the selected coin  
  
Usage: komodefi-cli swaps min-trading-vol <COIN>  
  
Arguments:  
 <COIN>     
  
Options:  
 -h, --help  Print help
```

**Example:**

```sh
komodefi-cli swaps min-trading-vol BTC  
Getting min trading vol: BTC  
coin: BTC  
volume: 0.0077
```

### swaps max-taker-vol

The `max-taker-vol` command provides the maximum volume of the given coin possible to trade. This command is implemented by  requesting the [`max_taker_vol` RPC API method](https://developers.komodoplatform.com/basic-docs/atomicdex-api-legacy/max_taker_vol.html).

```sh
komodefi-cli swaps max-taker-vol --help  
Return the maximum available volume for buy/sell methods for selected coin. The result should be used as is for sell method or divided by price for buy method.  
  
Usage: komodefi-cli swaps max-taker-vol <COIN>  
  
Arguments:  
 <COIN>     
  
Options:  
 -h, --help  Print help
```

**Example:**

```sh
komodefi-cli swaps max-taker-vol KMD  
Getting max taker vol, KMD  
coin: KMD  
result: 1596.16
```

```sh
komodefi-cli swaps max-taker-vol BTC  
Getting max taker vol, BTC  
coin: BTC  
result: 0
```

### swaps trade-preimage (preimage)

The `trade-preimage` command provides a way to estimate the fee for trading a given pair of coins at a given price and volume that could presumably take place. This command is implemented by requesting the [`trade_preimage` RPC API method](https://developers.komodoplatform.com/basic-docs/atomicdex-api-legacy/trade_preimage.html). *Alias: `preimage`*

```sh
komodefi-cli swap trade-preimage --help  
Return the approximate fee amounts that are paid per the whole swap  
  
Usage: komodefi-cli swaps trade-preimage <--volume <VOLUME>|--max> <BASE> <REL> <METHOD> <PRICE>  
  
Arguments:  
 <BASE>    Base currency of the request  
 <REL>     Rel currency of the request  
 <METHOD>  Price in rel the user is willing to pay per one unit of the base coin [possible values: set-price, buy, sell]  
 <PRICE>   Price in rel the user is willing to pay per one unit of the base coin  
  
Options:  
 -v, --volume <VOLUME>  Amount the user is willing to trade; ignored if max = true and swap_method = setprice, otherwise, it must be set  
 -m, --max              Whether to return the maximum available volume for setprice method; must not be set or false if swap_method is buy or sell  
 -h, --help             Print help
```

**Example:**

```sh
komodefi-cli swap trade-preimage --volume 1590 KMD BTC sell 0.00000859  
Getting trade preimage  
base_coin_fee: coin: KMD, amount: 0.00001, paid_from_trading_vol: false  
rel_coin_fee: coin: BTC, amount: 0.000050, paid_from_trading_vol: true  
total_fee:    
│ coin │ amount   │ required_balance │  
│ BTC  │ 0.000050 │ 0                │  
│ KMD  │ 1.84     │ 1.84             │
```

## Cancelling orders

Each of the following commands is designed to cancel one or more orders

```sh
komodefi-cli cancel  
Cancel one or many orders  
  
Usage: komodefi-cli cancel <COMMAND>  
  
Commands:  
 order, -o    Cancels certain order by uuid  
 all, -a      Cancels all orders of current node  
 by-pair, -p  Cancels all orders of specific pair [aliases: pair]  
 by-coin, -c  Cancels all orders using the coin ticker as base or rel [aliases: coin]  
 help         Print this message or the help of the given subcommand(s)  
  
Options:  
 -h, --help  Print help
```

### cancel order

The `cancel order` command cancels the given order using the [`cancel_order` RPC API method](https://developers.komodoplatform.com/basic-docs/atomicdex-api-legacy/cancel_order.html).

```sh
komodefi-cli cancel order --help  
Cancels certain order by uuid  
  
Usage: komodefi-cli cancel {order|-o} <UUID>  
  
Arguments:  
 <UUID>  Order identifier  
  
Options:  
 -h, --help  Print help
```

**Example:**

```sh
komodefi-cli orders book DOC MARTY --uuids  
Getting orderbook, base: DOC, rel: MARTY  
     Volume: DOC Price: MARTY     Uuid                                    
*          21.00 10.00000000      077b68d9-0e71-4a35-9b3f-c3cfe5b57310    
            0.14 1.00000000       0e549623-fead-4645-9c6c-00877b50bac2    
     94974264.87 1.00000000       6e0b356e-abbf-46e3-8a0c-4a19e6a88199    
- -------------- ---------------- ------------------------------------    
     94898462.11 1.00000000       390c85a5-7709-4c1f-a1bc-690412832bf6    
            1.94 1.00000000       fbd51c38-f3a7-42c5-aa3a-c52938188086    
*           3.00 0.33333333       797c0456-7d99-4295-8ee6-055e784b04cf

komodefi-cli cancel order 077b68d9-0e71-4a35-9b3f-c3cfe5b57310  
Cancelling order: 077b68d9-0e71-4a35-9b3f-c3cfe5b57310  
Order cancelled: Success
```

### cancel all

The `cancel all` command cancels all order created by the running `mm2` instance orders the [`cancel_all` RPC API method](https://developers.komodoplatform.com/basic-docs/atomicdex-api-legacy/cancel_all_orders.html).

```sh
komodefi-cli cancel all --help  
Cancels all orders of current node  
  
Usage: komodefi-cli cancel {all|-a}  
  
Options:  
 -h, --help  Print help
```

**Example:**

```sh
komodefi-cli cancel all  
Cancelling all orders  
Cancelled: d550c9d5-eedb-4069-ba0a-524d9346acda, 797c0456-7d99-4295-8ee6-055e784b04cf, 1fc0df20-9c21-461a-ad78-a4b37d4ab336
```

### cancel by-pair (pair)

The `cancel by-pair` command cancels all orders that matches by the given BASE and REL using the  [`cancel_all` RPC API method](https://developers.komodoplatform.com/basic-docs/atomicdex-api-legacy/cancel_all_orders.html). Sell operation is meant. *Alias: `pair`*

```sh
komodefi-cli cancel pair --help  
Cancels all orders of specific pair  
  
Usage: komodefi-cli cancel {by-pair|-p} <BASE> <REL>  
  
Arguments:  
 <BASE>  Base coin of the pair  
 <REL>   Rel coin of the pair  
  
Options:  
 -h, --help  Print help
```

**Example:**

```sh
komodefi-cli orders book DOC MARTY --uuids  
Getting orderbook, base: DOC, rel: MARTY  
     Volume: DOC Price: MARTY     Uuid                                    
*           1.00 3.00000000       72ad0098-a684-4ac1-925b-9b7155faa22a    
            0.14 1.00000000       0e549623-fead-4645-9c6c-00877b50bac2    
     94974264.87 1.00000000       36c3456d-7f69-4818-940a-72d5465217bd    
- -------------- ---------------- ------------------------------------    
     94898462.11 1.00000000       205fb457-f693-421b-ae16-48f63b996ad5    
            1.94 1.00000000       fbd51c38-f3a7-42c5-aa3a-c52938188086    
*           3.00 0.33333333       2018ece4-0210-4f07-8cff-eed811f17ded

komodefi-cli cancel pair DOC MARTY  
Cancelling by pair, base: DOC, rel: MARTY  
Cancelled: 72ad0098-a684-4ac1-925b-9b7155faa22a
```

### cancel by-coin (coin)

The `cancel by-coin` command cancels all orders corresponding to a given COIN in which this coin is set as a base or relative coin. This command is implemented by requesting the  [`cancel_all` RPC API method](https://developers.komodoplatform.com/basic-docs/atomicdex-api-legacy/cancel_all_orders.html) *Alias: `coin`*

```sh
komodefi-cli cancel coin --help  
Cancels all orders using the coin ticker as base or rel  
  
Usage: komodefi-cli cancel {by-coin|-c} <TICKER>  
  
Arguments:  
 <TICKER>  Order is cancelled if it uses ticker as base or rel  
  
Options:  
 -h, --help  Print help
```


**Example:**

```sh
komodefi-cli orders book DOC MARTY --uuids  
Getting orderbook, base: DOC, rel: MARTY  
     Volume: DOC Price: MARTY     Uuid                                    
*           1.00 3.00000000       22532835-8d93-4484-bb6b-e01be0acbde0    
            0.14 1.00000000       0e549623-fead-4645-9c6c-00877b50bac2    
     94974264.87 1.00000000       36c3456d-7f69-4818-940a-72d5465217bd    
- -------------- ---------------- ------------------------------------    
     94898462.11 1.00000000       205fb457-f693-421b-ae16-48f63b996ad5    
            1.94 1.00000000       fbd51c38-f3a7-42c5-aa3a-c52938188086    
*           3.00 0.33333333       2018ece4-0210-4f07-8cff-eed811f17ded

komodefi-cli cancel coin MARTY  
Cancelling by coin: MARTY  
Cancelled: 22532835-8d93-4484-bb6b-e01be0acbde0, 2018ece4-0210-4f07-8cff-eed811f17ded
```

## Utility

The utility provides both work with pubkey blacklists and obtaining avarage  MTP of electrum servers. * Aliases: `util`, `pubkeys`, `pubkey`*

```sh
komodefi-cli util  
Utility commands  
  
Usage: komodefi-cli utility <COMMAND>  
  
Commands:  
 ban-pubkey           Bans the given pubkey ignoring its order matching messages and preventing its orders from displaying in the orderbook. Use the secp256k1 pubkey without prefix for this method input [aliases: ban]  
 list-banned-pubkeys  Returns a list of public keys of nodes that are banned from interacting with the node executing the method [aliases: list, ban-list, list-banned]  
 unban-pubkeys        Remove all currently banned pubkeys from ban list, or specific pubkeys [aliases: unban]  
 get-current-mtp      Returns the Median Time Past (MTP) from electrum servers for UTXO coins [aliases: current-mtp, mtp]  
 help                 Print this message or the help of the given subcommand(s)  
  
Options:  
 -h, --help  Print help
```
### pubkey ban-pubkey (ban)

The `ban-pubkey` command bans the given pubkey ignoring its order matching messages and preventing its orders from displaying in the orderbook using the [`ban_pubkey` RPC API method](https://developers.komodoplatform.com/basic-docs/atomicdex-api-legacy/ban_pubkey.html). *Alias: `ban`*

```sh
komodefi-cli pubkey ban --help  
Bans the given pubkey ignoring its order matching messages and preventing its orders from displaying in the orderbook. Use the secp256k1 pubkey without prefix for this method input  
  
Usage: komodefi-cli utility ban-pubkey --pubkey <PUBKEY> --reason <REASON>  
  
Options:  
 -p, --pubkey <PUBKEY>  Pubkey to ban  
 -r, --reason <REASON>  Reason of banning  
 -h, --help             Print help
```

**Example:**

```sh
komodefi-cli order book KMD BTC --publics  
Getting orderbook, base: KMD, rel: BTC  
     Volume: KMD Price: BTC       Public                                                                
        19458.97 0.00004000       038f5fd70e8f97942913ceae60365c9a8ad26fa28733d1885c710b9036b6bffbab    
         3679.85 0.00000837       03de96cb66dcfaceaa8b3d4993ce8914cd5fe84e3fd53cefdae45add8032792a12    
...

komodefi-cli pubkey ban -p 8f5fd70e8f97942913ceae60365c9a8ad26fa28733d1885c710b9036b6bffbab --reason "too many failed trades"  
Banning pubkey: 8f5fd70e8f97942913ceae60365c9a8ad26fa28733d1885c710b9036b6bffbab  
Status: Success
...

komodefi-cli order book KMD BTC --publics  
Getting orderbook, base: KMD, rel: BTC  
     Volume: KMD Price: BTC       Public                                                                
         3695.15 0.00000835       03de96cb66dcfaceaa8b3d4993ce8914cd5fe84e3fd53cefdae45add8032792a12    
```

The orders for banned pubkey have disappeared from the order book.

### pubkey list-banned-pubkeys (list)

The `list-banned-pubkeys` lists previously banned pubkeys using the [`list_banned_pubkeys` RPC API methods](https://developers.komodoplatform.com/basic-docs/atomicdex-api-legacy/list_banned_pubkeys.html). *Alias: `list`, `ban-list`, `list-banned`*

```sh
komodefi-cli pubkeys list --help  
Returns a list of public keys of nodes that are banned from interacting with the node executing the method  
  
Usage: komodefi-cli utility list-banned-pubkeys  
  
Options:  
 -h, --help  Print help
```

**Example:**

```sh
komodefi-cli pubkeys list  
Getting list of banned pubkeys  
│ pubkey                                                           │ reason │ comment                │  
│ 8f5fd70e8f97942913ceae60365c9a8ad26fa28733d1885c710b9036b6bffbab │ manual │ too many failed trades │  
│ 5d81c96aa4269c5946c0bd8dad7785ae0f4f595e7aea2ec4f8fe71f77ebf74a9 │ manual │ test ban               │  
│ 1bb83b58ec130e28e0a6d5d2acf2eb01b0d3f1670e021d47d31db8a858219da8 │ manual │ one more test ban      │
```
### pubkey unban-pubkeys (unban)

The `unban-pubkeys` command unbans whether the all previously banned pubkey or a given one using the [`unban_pubkey` RPC API method](https://developers.komodoplatform.com/basic-docs/atomicdex-api-legacy/unban_pubkeys.html#unban-pubkeys). *Alias: `ban`*

```sh
komodefi-cli pubkey unban --help  
Remove all currently banned pubkeys from ban list, or specific pubkeys  
  
Usage: komodefi-cli utility unban-pubkeys <--all|--pubkey <PUBKEY>>  
  
Options:  
 -a, --all              Whether to unban all pubkeys  
 -p, --pubkey <PUBKEY>  Pubkey to unban  
 -h, --help             Print help
```

**Example:**

Unban a certain pubkey

```sh
komodefi-cli pubkey unban -p 8f5fd70e8f97942913ceae60365c9a8ad26fa28733d1885c710b9036b6bffbab    
Unbanning pubkeys  
still_banned: none  
unbanned: 8f5fd70e8f97942913ceae60365c9a8ad26fa28733d1885c710b9036b6bffbab(manually "too many failed trades")  
were_not_banned: none
```

Unban all pubkeys

```sh
komodefi-cli pubkey unban --all  
Unbanning pubkeys  
still_banned: none  
unbanned: 5d81c96aa4269c5946c0bd8dad7785ae0f4f595e7aea2ec4f8fe71f77ebf74a9(manually "test ban"), 1bb83b58ec130e28e0a6d5d2acf2eb01b0d3f1670e021d47d31db8a858219da8(manually "one more test ban"), 8f5fd70e8f97942913ceae60365c9a8ad26fa28733d  
1885c710b9036b6bffbab(manually "too many failed trades")  
were_not_banned: none
```

### utild get-current-mtp (mtp)

The `get-current-mtp` command returns the Median Time Past (MTP) from electrum servers for UTXO coins using the [`get_current_mtp` RPC API method](https://developers.komodoplatform.com/basic-docs/atomicdex-api-20-dev/get_current_mtp.html). This information is useful for debugging, specifically in cases where an electrum server has been misconfigured. *Aliases: `current-mtp`, `mtp`*

```sh
komodefi-cli utility mtp --help  
Returns the Median Time Past (MTP) from electrum servers for UTXO coins  
  
Usage: komodefi-cli utility get-current-mtp <COIN>  
  
Arguments:  
 <COIN>  A compatible (UTXO) coin's ticker  
  
Options:  
 -h, --help  Print help
```

**Example:**

```sh
komodefi-cli util mtp DOC  
Getting current MTP, coin: DOC  
Current mtp: 23-09-18 08:34:40
```
## Message signing

The Komodo defi platform provides options for creating a signature on a given message or verifying a message signature. This facility could be used to prove ownership of an address.

### message sign

The `sign` command provides a signature on a given message using the [`sign_message` RPC API message](https://developers.komodoplatform.com/basic-docs/atomicdex-api-20/message_signing.html#message-signing).

```sh
komodefi-cli message sign --help  
If your coins file contains the correct message prefix definitions, you can sign message to prove ownership of an address  
  
Usage: komodefi-cli message {sign|-s} --coin <COIN> --message <MESSAGE>  
  
Options:  
 -c, --coin <COIN>        The coin to sign a message with  
 -m, --message <MESSAGE>  The message you want to sign  
 -h, --help               Print help
```

**Example:**

```sh
komodefi-cli message sign --coin DOC --message "test message"  
Signing message  
signature: INAWPG9a6vKhbcwhpb6i8Zjg/0sZ30LOcGcW1TmZBDiCfcPIVYvQ5hjWez9wUj4UT6Gc+M2Ky4aKSgIJggUNqTI=
```

### message verify

The `verify` command verifies the signature of a message previously created on a given coin and address.

```sh
komodefi-cli message verify --help  
Verify message signature  
  
Usage: komodefi-cli message {verify|-v} --coin <COIN> --message <MESSAGE> --signature <SIGNATURE> --address <ADDRESS>  
  
Options:  
 -c, --coin <COIN>            The coin to sign a message with  
 -m, --message <MESSAGE>      The message input via the sign_message method sign  
 -s, --signature <SIGNATURE>  The signature generated for the message  
 -a, --address <ADDRESS>      The address used to sign the message  
 -h, --help                   Print help
```

**Example:**

```sh
komodefi-cli message verify -c DOC --message "test message" --signature INAWPG9a6vKhbcwhpb6i8Zjg/0sZ30LOcGcW1TmZBDiCfcPIVYvQ5hjWez9wUj4UT6Gc+M2Ky4aKSgIJggUNqTI= --address RPFGrvJWjSYN4qYvcXsECW1H  
oHbvQjowZM  
Verifying message  
is valid: valid
```

## Network

The Network commands are designed to obtain komodo network characteristics that may be useful for debugging purposes.

```sh
komodefi-cli network  
Network commands  
  
Usage: komodefi-cli network <COMMAND>  
  
Commands:  
 get-gossip-mesh         Return an array of peerIDs added to a topics' mesh for each known gossipsub topic [aliases: gossip-mesh]  
 get-relay-mesh          Return a list of peerIDs included in our local relay mesh [aliases: relay-mesh]  
 get-gossip-peer-topics  Return a map of peerIDs to an array of the topics to which they are subscribed [aliases: peer-topics]  
 get-gossip-topic-peers  Return a map of topics to an array of the PeerIDs which are subscribers [aliases: topic-peers]  
 get-my-peer-id          Return your unique identifying Peer ID on the network [aliases: my-peer-id]  
 get-peers-info          Return all connected peers with their multiaddresses [aliases: peers-info]  
 help                    Print this message or the help of the given subcommand(s)  
  
Options:  
 -h, --help  Print help
```

### network get-gossip-mesh (gossip-mesh)

The `get-gossip-mesh` returns an array of peerIDs added to a topics' mesh for each known gossipsub topic using the [` get_gossip_mesh` RPC API method]. *Alias: `gossip-mesh`*

```sh
komodefi-cli network gossip-mesh --help  
Return an array of peerIDs added to a topics' mesh for each known gossipsub topic  
  
Usage: komodefi-cli network get-gossip-mesh  
  
Options:  
 -h, --help  Print help
```

**Example: **

```sh
komodefi-cli network gossip-mesh  
Getting gossip mesh  
gossip_mesh:    
orbk/DOC:KMD: empty  
orbk/BTC:KMD: empty
```
### network get-relay-mesh (relay-mesh)

The `get-relay-mesh` command returns a list of peerIDs included in our local relay mesh using [`get_relay_mesh` RPC API method](https://developers.komodoplatform.com/basic-docs/atomicdex-api-legacy/get_gossip_mesh.html). *Alias: `relay-mesh`*

```sh
komodefi-cli network relay-mesh --help  
Return a list of peerIDs included in our local relay mesh  
  
Usage: komodefi-cli network get-relay-mesh  
  
Options:  
 -h, --help  Print help
```

**Example:**

```sh
komodefi-cli network relay-mesh  
Getting relay mesh  
relay_mesh:    
12D3KooWDsFMoRoL5A4ii3UonuQZ9Ti2hrc7PpytRrct2Fg8GRq9  
12D3KooWBXS7vcjYGQ5vy7nZj65FicpdxXsavPdLYB8gN7Ai3ruA
```

### network get-gossip-peer-topics (peer-topics)

The `get-gossip-peer-topics` returns a map of peerIDs to an array of the topics to which they are subscribed using the [` get_gossip_peer_topics` RPC API method](https://developers.komodoplatform.com/basic-docs/atomicdex-api-legacy/get_gossip_peer_topics.html). *Alias: `peer-topics`*

```sh
komodefi-cli network peer-topics --help  
Return a map of peerIDs to an array of the topics to which they are subscribed  
  
Usage: komodefi-cli network get-gossip-peer-topics  
  
Options:  
 -h, --help  Print help
```

**Example:**

```sh
komodefi-cli network peer-topics    
Getting gossip peer topics  
gossip_peer_topics:    
12D3KooWSmizY35qrfwX8qsuo8H8qrrvDjXBTMRBfeYsRQoybHaA: empty  
12D3KooWEaZpH61H4yuQkaNG5AsyGdpBhKRppaLdAY52a774ab5u: empty  
12D3KooWCSidNncnbDXrX5G6uWdFdCBrMpaCAqtNxSyfUcZgwF7t: empty  
12D3KooWDgFfyAzbuYNLMzMaZT9zBJX9EHd38XLQDRbNDYAYqMzd: empty  
12D3KooWDsFMoRoL5A4ii3UonuQZ9Ti2hrc7PpytRrct2Fg8GRq9: empty  
12D3KooWBXS7vcjYGQ5vy7nZj65FicpdxXsavPdLYB8gN7Ai3ruA: empty  
12D3KooWEsuiKcQaBaKEzuMtT6uFjs89P1E8MK3wGRZbeuCbCw6P: empty  
12D3KooWMsfmq3bNNPZTr7HdhTQvxovuR1jo5qvM362VQZorTk3F: empty
```

### network get-gossip-topic-peers (topic-peers)

The `get-gossip-topic-peers` command returns a map of topics to an array of the PeerIDs which are subscribers using the [` get_gossip_topic_peers` RPC API method](https://developers.komodoplatform.com/basic-docs/atomicdex-api-legacy/get_gossip_topic_peers.html). *Alias: `topic-peers`*

```sh
omodefi-cli network get-gossip-topic-peers --help  
Return a map of topics to an array of the PeerIDs which are subscribers  
  
Usage: komodefi-cli network get-gossip-topic-peers  
  
Options:  
 -h, --help  Print help
```

**Example:**

```sh
komodefi-cli network get-gossip-topic-peers    
Getting gossip topic peers  
gossip_topic_peers: empty
```

### network get-my-peer-id (my-peer-id)

The `get-my-peer-id` command returns the unique identifying Peer ID corresponding to the running `mm2` node on the network. *Alias: `my-peer-id`*

```sh
komodefi-cli network my-peer-id --help  
Return your unique identifying Peer ID on the network  
  
Usage: komodefi-cli network get-my-peer-id  
  
Options:  
 -h, --help  Print help
```

**Example:**

```sh
komodefi-cli network my-peer-id  
Getting my peer id  
12D3KooWA8XJs1HocoDcb28sNYk79UX4ibuBJuHSR84wdWBQPiCr
```
### network get-peers-info (peers-info)

The `get-peers-info` command returns all connected peers with their multiaddresses using the [` get_peers_info`](https://developers.komodoplatform.com/basic-docs/atomicdex-api-legacy/get_peers_info.html).

```sh
komodefi-cli network peers-info --help  
Return all connected peers with their multiaddresses  
  
Usage: komodefi-cli network get-peers-info  
  
Options:  
 -h, --help  Print help
```

**Example:**

```sh
komodefi-cli network peers-info    
Getting peers info  
peers_info:    
12D3KooWEaZpH61H4yuQkaNG5AsyGdpBhKRppaLdAY52a774ab5u: /ip4/46.4.78.11/tcp/38890/p2p/12D3KooWEaZpH61H4yuQkaNG5AsyGdpBhKRppaLdAY52a774ab5u  
12D3KooWMsfmq3bNNPZTr7HdhTQvxovuR1jo5qvM362VQZorTk3F: /ip4/2.56.154.200/tcp/38890/p2p/12D3KooWMsfmq3bNNPZTr7HdhTQvxovuR1jo5qvM362VQZorTk3F  
12D3KooWDsFMoRoL5A4ii3UonuQZ9Ti2hrc7PpytRrct2Fg8GRq9: /ip4/148.113.1.52/tcp/38890,/ip4/148.113.1.52/tcp/38890/p2p/12D3KooWDsFMoRoL5A4ii3UonuQZ9Ti2hrc7PpytRrct2Fg8GRq9  
...
```

## Tracking seed-node version statistics

The possibility to keep version statistics is provided by tracking nodes that are set up on the `mm2` instance and controlled by version statistics commands. *Aliases: `stat`, `vstat`*

```sh
komodefi-cli stat  
Version statistic commands  
  
Usage: komodefi-cli version-stat <COMMAND>  
  
Commands:  
 add-node, -a        Adds a Node's name, IP address and PeerID to a local database to track which version of MM2 it is running. Note: To allow collection of version stats, added nodes must open port 38890 [aliases: add, add-node-to-ver  
sion-stat]  
 remove-node, -r     Removes a Node (by name) from the local database which tracks which version of MM2 it is running [aliases: remove, remove-node-from-version-stat]  
 start-collect, -s   Initiates storing version statistics for nodes previously registered via the add-node command [aliases: start, start-version-stat-collection]  
 stop-collect, -S    Stops the collection of version stats at the end of the current loop interval [aliases: stop, stop-version-stat-collection]  
 update-collect, -u  Updates the polling interval for version stats collection. Note: the new interval will take effect after the current interval loop has completed. [aliases: update, update-version-stat-collection]  
 help                Print this message or the help of the given subcommand(s)  
  
Options:  
 -h, --help  Print help
```

### version-stat add-node-to-version-stat (add)

The `add-node-to-version-stat` command provides setting up th version statistics node to a local database to track which version of MM2 it is running using the [` add_node_to_version_stat` RPC API method].  *Alias: `add`, `add-node`*

```
komodefi-cli stat add --help  
Adds a Node's name, IP address and PeerID to a local database to track which version of MM2 it is running. Note: To allow collection of version stats, added nodes must open port 38890  
  
Usage: komodefi-cli version-stat {add-node|-a} --name <NAME> --address <ADDRESS> --peer-id <PEER_ID>  
  
Options:  
 -n, --name <NAME>        The name assigned to the node, arbitrary identifying string, such as "seed_alpha" or "dragonhound_DEV"  
 -a, --address <ADDRESS>  The Node's IP address or domain names  
 -p, --peer-id <PEER_ID>  The Peer ID can be found in the MM2 log file after a connection has been initiated  
 -h, --help               Print help
```

**Example:**

```sh
komodefi-cli stat add --name "test" --address "25.145.122.43" --peer-id 12D3KooWEsuiKcQaBaKEzuMtT6uFjs89P1E8MK3wGRZbeuCbCw6P  
Adding stat collection node  
Add node to version stat: Success
```

### version-stat remove-node-from-version-stat (remove)

The `remove-node-from-version-stat` command removes the given node from the list of statistics tracking nodes using the [`remove_node_from_version_stat` RPC API method](https://developers.komodoplatform.com/basic-docs/atomicdex-api-20/remove_node_from_version_stat.html). *Alias: `remove`, `remove-node`*

```sh
komodefi-cli stat remove-node --help  
Removes a Node (by name) from the local database which tracks which version of MM2 it is running  
  
Usage: komodefi-cli version-stat {remove-node|-r} <NAME>  
  
Arguments:  
 <NAME>  The name assigned to the node, arbitrary identifying string, such as "seed_alpha" or "dragonhound_DEV"  
  
Options:  
 -h, --help  Print help
```

**Example:**

```sh
komodefi-cli stat remove-node test  
Removing stat collection node  
Remove node from version stat: Success
```

### version-stat start-version-stat-collection (start)

The `start-version-stat-collection` command initiates gathering of version statistics on nodes previously registered by the `add-node` command using the [` start_version_stat_collection` RPC API] method.  The statistics accumulation is done at polling intervals. *Aliases: `start`, `start-collect`.


```sh
komodefi-cli stat start --help  
Initiates storing version statistics for nodes previously registered via the add-node command  
  
Usage: komodefi-cli version-stat {start-collect|-s} <INTERVAL>  
  
Arguments:  
 <INTERVAL>  Polling rate (in seconds) to check node versions  
  
Options:  
 -h, --help  Print help
```

**Example:**

```sh
komodefi-cli stat start 5  
Starting stat collection  
Start stat collection: Success
```

### version-stat stop-version-stat-collection (stop)

The `stop-version-stat-collection` stops gathering of version statistcis using the [`stop_version_stat_collection` RPC API method](https://developers.komodoplatform.com/basic-docs/atomicdex-api-20/stop_version_stat_collection.html). *Aliases: `stop`, `stop-collect`*

```sh
komodefi-cli stat stop --help  
Stops the collection of version stats at the end of the current loop interval  
  
Usage: komodefi-cli version-stat {stop-collect|-S}  
  
Options:  
 -h, --help  Print help
```

**Example:**

```sh
komodefi-cli stat stop  
Stopping stat collection  
Stop stat collection: Success
```

### version-stat update-version-stat-collection (update)

The `update-version-stat-collection` updates the polling interval for version stats collection using the [`update_version_stat_collection` RPC API method].

```sh
komodefi-cli stat update --help  
Updates the polling interval for version stats collection. Note: the new interval will take effect after the current interval loop has completed.  
  
Usage: komodefi-cli version-stat {update-collect|-u} <INTERVAL>  
  
Arguments:  
 <INTERVAL>  Polling rate (in seconds) to check node versions  
  
Options:  
 -h, --help  Print help
```

**Example:**

```sh
komodefi-cli stat update 10  
Updating stat collection  
Update stat collection: Success
```

## Task managing

The komodo defi platform protocol provides a number of asynchronous commands. The current implementation of `komodefi-cli` includes an asynchronous ZHTLC coin enable command. This command returns the `task_id` associated with the command that is running in the background.  The following commands provide task management functionality.

```sh
komodefi-cli task  
Tracking the status of long-running commands  
  
Usage: komodefi-cli task <COMMAND>  
  
Commands:  
 status  Get status of task  
 cancel  Cancel task  
 help    Print this message or the help of the given subcommand(s)  
  
Options:  
 -h, --help  Print help
```

### task status zcoin

The `status` command gets a status of the background activity by the task_id using the [`task_enable_z_coin_status` RPC API method](https://developers.komodoplatform.com/basic-docs/atomicdex-api-20-dev/zhtlc_coins.html#task-enable-z-coin-status)

```sh
komodefi-cli task status zcoin --help  
Get zcoin enabling status  
  
Usage: komodefi-cli task status zcoin <TASK_ID>  
  
Arguments:  
 <TASK_ID>     
  
Options:  
 -h, --help  Print help
```

**Example**

```sh
komodefi-cli coin enable ZOMBIE  
Enabling coin: ZOMBIE  
Enabling zcoin started, task_id: 2
...
komodefi-cli task status zcoin 2  
Getting enable zcoin task status  
In progress: Activating coin
...
komodefi-cli task status zcoin 2  
Getting enable zcoin task status  
Error: Error on platform coin ZOMBIE creation: All the current light clients are unavailable.
```
