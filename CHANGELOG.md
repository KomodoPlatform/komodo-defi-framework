## {inc-release}

**Features:**

**Enhancements/Fixes:**
- CI/CD workflow logics are improved [#1736](https://github.com/KomodoPlatform/atomicDEX-API/pull/1736)
- Project root is simplified/refactored [#1738](https://github.com/KomodoPlatform/atomicDEX-API/pull/1738)
- Created base image to provide more glibc compatible pre-built binaries for linux [#1741](https://github.com/KomodoPlatform/atomicDEX-API/pull/1741)
- Set default log level as "info" [#1747](https://github.com/KomodoPlatform/atomicDEX-API/pull/1747)

## v1.0.1-beta - 2023-03-17

**Features:**
- NFT integration `WIP` [#900](https://github.com/KomodoPlatform/atomicDEX-API/issues/900)
  - NFT integration PoC added. Includes ERC721 support for ETH and BSC [#1652](https://github.com/KomodoPlatform/atomicDEX-API/pull/1652)
  - Withdraw ERC1155 and EVM based chains support added for NFT PoC [#1704](https://github.com/KomodoPlatform/atomicDEX-API/pull/1704)
- Swap watcher nodes [#1431](https://github.com/KomodoPlatform/atomicDEX-API/issues/1431)
  - Watcher rewards for ETH swaps were added [#1658](https://github.com/KomodoPlatform/atomicDEX-API/pull/1658)
- Cosmos integration `WIP` [#1432](https://github.com/KomodoPlatform/atomicDEX-API/issues/1432)
  - `ibc_withdraw`, `ibc_chains` and `ibc_transfer_channels` RPC methods were added [#1636](https://github.com/KomodoPlatform/atomicDEX-API/pull/1636)
- Lightning integration `WIP` [#1045](https://github.com/KomodoPlatform/atomicDEX-API/issues/1045)
  - [rust-lightning](https://github.com/lightningdevkit/rust-lightning) was updated to [v0.0.113](https://github.com/lightningdevkit/rust-lightning/releases/tag/v0.0.113) in [#1655](https://github.com/KomodoPlatform/atomicDEX-API/pull/1655)
  - Channel `current_confirmations` and `required_confirmations` were added to channel details RPCs in [#1655](https://github.com/KomodoPlatform/atomicDEX-API/pull/1655)
  - `Uuid` is now used for internal channel id instead of `u64` [#1655](https://github.com/KomodoPlatform/atomicDEX-API/pull/1655)
  - Some unit tests were added for multi path payments in [#1655](https://github.com/KomodoPlatform/atomicDEX-API/pull/1655)
  - Some unit tests for claiming swaps on-chain for closed channels were added in [#1655](https://github.com/KomodoPlatform/atomicDEX-API/pull/1655), these unit tests are currently failing.
  - `protocol_info` fields are now used to check if a lightning order can be matched or not in [#1655](https://github.com/KomodoPlatform/atomicDEX-API/pull/1655)
  - 2 issues discovered while executing a KMD/LNBTC swap were fixed in [#1655](https://github.com/KomodoPlatform/atomicDEX-API/pull/1655), these issues were:
    - When electrums were down, if a channel was closed, the channel closing transaction wasn't broadcasted. A check for a network error to rebroadcast the tx after a delay was added.
    - If an invoice payment failed, retring paying the same invoice would cause the payment to not be updated to successful in the DB even if it were successful. A method to update the payment in the DB was added to fix this.
  - `mm2_git` crate was added to provide an abstraction layer on Git for doing query/parse operations over the repositories [#1636](https://github.com/KomodoPlatform/atomicDEX-API/pull/1636)

**Enhancements/Fixes:**
- Use `env_logger` to achieve flexible log filtering [#1725](https://github.com/KomodoPlatform/atomicDEX-API/pull/1725)
- IndexedDB Cursor can now iterate over the items step-by-step [#1678](https://github.com/KomodoPlatform/atomicDEX-API/pull/1678)
- Uuid lib was update from v0.7.4 to v1.2.2 in [#1655](https://github.com/KomodoPlatform/atomicDEX-API/pull/1655)
- A bug was fixed in [#1706](https://github.com/KomodoPlatform/atomicDEX-API/pull/1706) where EVM swap transactions were failing if sent before the approval transaction confirmation.
- Tendermint account sequence problem due to running multiple instances were fixed in [#1694](https://github.com/KomodoPlatform/atomicDEX-API/pull/1694)
- Maker/taker pubkeys were added to new columns in `stats_swaps` table in [#1665](https://github.com/KomodoPlatform/atomicDEX-API/pull/1665) and [#1717](https://github.com/KomodoPlatform/atomicDEX-API/pull/1717)
- Get rid of unnecessary / old dependencies: `crossterm`, `crossterm_winapi`, `mio 0.7.13`, `miow`, `ntapi`, `signal-hook`, `signal-hook-mio` in [#1710](https://github.com/KomodoPlatform/atomicDEX-API/pull/1710)
- A bug that caused EVM swap payments validation to fail because the tx was not available yet in the RPC node when calling `eth_getTransactionByHash` was fixed in [#1716](https://github.com/KomodoPlatform/atomicDEX-API/pull/1716). `eth_getTransactionByHash` in now retried in `wait_for_confirmations` until tx is found in the RPC node, this makes sure that the transaction is returned from `eth_getTransactionByHash` later when validating.
- CI/CD migrated from Azure to Github runners [#1699](https://github.com/KomodoPlatform/atomicDEX-API/pull/1699)
- CI tests are much stabilized [#1699](https://github.com/KomodoPlatform/atomicDEX-API/pull/1699)
- Integration and unit tests are seperated on CI stack [#1699](https://github.com/KomodoPlatform/atomicDEX-API/pull/1699)
- Codebase is updated in linting rules at wasm and test stack [#1699](https://github.com/KomodoPlatform/atomicDEX-API/pull/1699)
- `crossbeam` bumped to `0.8` from `0.7` [#1699](https://github.com/KomodoPlatform/atomicDEX-API/pull/1699)
- Un-used/Unstable parts of mm2 excluded from build outputs which avoids mm2 runtime from potential UB [#1699](https://github.com/KomodoPlatform/atomicDEX-API/pull/1699)
- Build time optimizations applied such as sharing generics instead of duplicating them in binary (which reduces output sizes) [#1699](https://github.com/KomodoPlatform/atomicDEX-API/pull/1699)
- `RUSTSEC-2020-0036`, `RUSTSEC-2021-0139` and `RUSTSEC-2023-0018` resolved [#1699](https://github.com/KomodoPlatform/atomicDEX-API/pull/1699)
- Enabled linting checks for wasm and test stack on CI [#1699](https://github.com/KomodoPlatform/atomicDEX-API/pull/1699)
- Release container base image updated to debian stable from ubuntu bionic [#1699](https://github.com/KomodoPlatform/atomicDEX-API/pull/1699)
- Fix dylib linking error of rusb [#1699](https://github.com/KomodoPlatform/atomicDEX-API/pull/1699)
- `OperationFailure::Other` error was expanded. New error variants were matched with `HwRpcError`, so error type will be `HwError`, not `InternalError` [#1719](https://github.com/KomodoPlatform/atomicDEX-API/pull/1719)
- RPC calls for evm chains was reduced in `wait_for_confirmations` function in [#1724](https://github.com/KomodoPlatform/atomicDEX-API/pull/1724)
- A possible endless loop in evm `wait_for_htlc_tx_spend` was fixed in [#1724](https://github.com/KomodoPlatform/atomicDEX-API/pull/1724)
- Wait time for taker fee validation was increased from 30 to 60 seconds in [#1730](https://github.com/KomodoPlatform/atomicDEX-API/pull/1730) to give the fee tx more time to appear in most nodes mempools.

## v1.0.0-beta - 2023-03-08

**Features:**
- ARRR integration [#927](https://github.com/KomodoPlatform/atomicDEX-API/issues/927):
  - Zcoin native mode support was added [#1438](https://github.com/KomodoPlatform/atomicDEX-API/pull/1438)
  - Multi lightwalletd servers support was added [#1472](https://github.com/KomodoPlatform/atomicDEX-API/pull/1472)
  - Allow passing Zcash params file path to activation request [#1538](https://github.com/KomodoPlatform/atomicDEX-API/pull/1538)
  - Checksum verification of Zcash params files was added  [#1549](https://github.com/KomodoPlatform/atomicDEX-API/pull/1549)
- Tendermint integration [#1432](https://github.com/KomodoPlatform/atomicDEX-API/issues/1432)
  - Tendermint HTLC implementation [#1454](https://github.com/KomodoPlatform/atomicDEX-API/pull/1454)
  - Tendermint swap support (POC level) [#1468](https://github.com/KomodoPlatform/atomicDEX-API/pull/1454)
  - Complete tendermint support for swaps and tx history implementation [#1526](https://github.com/KomodoPlatform/atomicDEX-API/pull/1526)
  - Improve rpc client rotation of tendermint [#1675](https://github.com/KomodoPlatform/atomicDEX-API/pull/1675)
- HD Wallet [#740](https://github.com/KomodoPlatform/atomicDEX-API/issues/740)
  - Implement Global HD account activation mode [#1512](https://github.com/KomodoPlatform/atomicDEX-API/pull/1512)
  - `mm2_rmd160` property was removed from the HD account table. Now, either Iguana or an HD account share the same HD account records [#1672](https://github.com/KomodoPlatform/atomicDEX-API/pull/1672)
- Hardware Wallet [#964](https://github.com/KomodoPlatform/atomicDEX-API/issues/964)
  - Implement TX history V2 for UTXO coins activated with a Hardware wallet [#1467](https://github.com/KomodoPlatform/atomicDEX-API/pull/1467)
  - Fix KMD withdraw with Trezor [#1628](https://github.com/KomodoPlatform/atomicDEX-API/pull/1628)
  - `task::get_new_address::*` RPCs were added to replace the legacy `get_new_address` RPC [#1672](https://github.com/KomodoPlatform/atomicDEX-API/pull/1672)
  - `trezor_connection_status` RPC was added to allow the GUI to poll the Trezor connection status [#1672](https://github.com/KomodoPlatform/atomicDEX-API/pull/1672)
- Simple Payment Verification [#1612](https://github.com/KomodoPlatform/atomicDEX-API/issues/1612)
  - Implement unit test for `Block header UTXO Loop` [#1519](https://github.com/KomodoPlatform/atomicDEX-API/pull/1519)
  - `SPV` with minimal storage requirements and fast block headers sync time was implemented [#1585](https://github.com/KomodoPlatform/atomicDEX-API/pull/1585)
  - Block headers storage was implemented for `IndexedDB` [#1644](https://github.com/KomodoPlatform/atomicDEX-API/pull/1644)
  - `SPV` was re-enabled in `WASM` [#1644](https://github.com/KomodoPlatform/atomicDEX-API/pull/1644)
- New RPCs
  - gui-auth and `enable_eth_with_tokens` `enable_erc20` RPCs were added [#1335](https://github.com/KomodoPlatform/atomicDEX-API/pull/1335)
  - `get_current_mtp` RPC was added [#1340](https://github.com/KomodoPlatform/atomicDEX-API/pull/1340)
  - `max_maker_vol` RPC was added [#1618](https://github.com/KomodoPlatform/atomicDEX-API/pull/1618)
- Lightning integration `WIP` [#1045](https://github.com/KomodoPlatform/atomicDEX-API/issues/1045)
  - [rust-lightning](https://github.com/lightningdevkit/rust-lightning) was updated to [v0.0.110](https://github.com/lightningdevkit/rust-lightning/releases/tag/v0.0.110) in [#1452](https://github.com/KomodoPlatform/atomicDEX-API/pull/1452)
  - Inbound channels details was added to SQL channels history in [#1339](https://github.com/KomodoPlatform/atomicDEX-API/pull/1339)
  - Blocking was fixed for sync rust-lightning functions that calls other I/O functions or that has mutexes that can be held for some time in [#1452](https://github.com/KomodoPlatform/atomicDEX-API/pull/1452)
  - Default fees are retrieved from rpc instead of config when starting lightning [#1452](https://github.com/KomodoPlatform/atomicDEX-API/pull/1452)
  - 0 confirmations channels feature was added in [#1452](https://github.com/KomodoPlatform/atomicDEX-API/pull/1452)
  - An `update_channel` RPC was added that updates a channel that is open without closing it in [#1452](https://github.com/KomodoPlatform/atomicDEX-API/pull/1452)
  - Lightning RPCs now use the `lightning::` namespace in [#1497](https://github.com/KomodoPlatform/atomicDEX-API/pull/1497)
  - `TakerFee` and `MakerPayment` swap messages were modified to include payment instructions for the other side, in the case of lightning this payment instructions is a lightning invoice [#1497](https://github.com/KomodoPlatform/atomicDEX-API/pull/1497)
  - `MakerPaymentInstructionsReceived`/`TakerPaymentInstructionsReceived` events are added to `MakerSwapEvent`/`TakerSwapEvent` in [#1497](https://github.com/KomodoPlatform/atomicDEX-API/pull/1497), for more info check this [comment](https://github.com/KomodoPlatform/atomicDEX-API/issues/1045#issuecomment-1410449770)
  - Lightning swaps were implemented in [#1497](https://github.com/KomodoPlatform/atomicDEX-API/pull/1497), [#1557
    ](https://github.com/KomodoPlatform/atomicDEX-API/pull/1557)
  - Lightning swap refunds were implemented in [#1592](https://github.com/KomodoPlatform/atomicDEX-API/pull/1592)
  - `MakerPaymentRefundStarted`, `TakerPaymentRefundStarted`, `MakerPaymentRefundFinished`, `TakerPaymentRefundFinished` events were added to swap error events in [#1592](https://github.com/KomodoPlatform/atomicDEX-API/pull/1592), for more info check this [comment](https://github.com/KomodoPlatform/atomicDEX-API/issues/1045#issuecomment-1410449770)
  - Enabling lightning now uses the task manager [#1513](https://github.com/KomodoPlatform/atomicDEX-API/pull/1513)
  - Disabling lightning coin or calling `stop` RPC now drops the `BackgroundProcessor` which persists the latest network graph and scorer to disk [#1513](https://github.com/KomodoPlatform/atomicDEX-API/pull/1513), [#1490](https://github.com/KomodoPlatform/atomicDEX-API/pull/1490)
  - `avg_blocktime` from platform/utxo coin is used for l2/lightning estimating of the number of blocks swap payments are locked for [#1606](https://github.com/KomodoPlatform/atomicDEX-API/pull/1606)
- MetaMask `WIP` [#1167](https://github.com/KomodoPlatform/atomicDEX-API/issues/1167)
  - Login with a MetaMask wallet [#1551](https://github.com/KomodoPlatform/atomicDEX-API/pull/1551)
  - Check if corresponding ETH chain is known by MetaMask wallet on coin activation using `wallet_switchEthereumChain` [#1674](https://github.com/KomodoPlatform/atomicDEX-API/pull/1674)
  - Refactor ETH/ERC20 withdraw taking into account that the only way to sign a transaction is to send it using `eth_sendTransaction` [#1674](https://github.com/KomodoPlatform/atomicDEX-API/pull/1674)
  - Extract address's public key using `eth_singTypedDataV4` [#1674](https://github.com/KomodoPlatform/atomicDEX-API/pull/1674)
  - Perform swaps with coins activated with MetaMask [#1674](https://github.com/KomodoPlatform/atomicDEX-API/pull/1674)
   
**Enhancements/Fixes:**
- Update `rust-web3` crate [#1674](https://github.com/KomodoPlatform/atomicDEX-API/pull/1674)
- Custom enum from stringify derive macro to derive From implementations for enums  [#1502](https://github.com/KomodoPlatform/atomicDEX-API/pull/1502)
- Validate that  `input_tx` is calling `'receiverSpend'` in `eth::extract_secret` [#1596](https://github.com/KomodoPlatform/atomicDEX-API/pull/1596)
- Validate all Swap parameters at the Negotiation stage [#1475](https://github.com/KomodoPlatform/atomicDEX-API/pull/1475)
- created safe number type castings [#1517](https://github.com/KomodoPlatform/atomicDEX-API/pull/1517)
- Improve `stop` functionality [#1490](https://github.com/KomodoPlatform/atomicDEX-API/pull/1490)
- A possible seednode p2p thread panicking attack due to `GetKnownPeers` msg was fixed in [#1445](https://github.com/KomodoPlatform/atomicDEX-API/pull/1445)
- NAV `cold_staking` script type was added to fix a problem in NAV tx history in [#1466](https://github.com/KomodoPlatform/atomicDEX-API/pull/1466)
- SPV was temporarily disabled in WASM in [#1479](https://github.com/KomodoPlatform/atomicDEX-API/pull/1479)
- `BTC-segwit` swap locktimes was fixed in [#1548](https://github.com/KomodoPlatform/atomicDEX-API/pull/1548) by using orderbook ticker instead of ticker in swap locktimes calculations.
- BTC block headers deserialization was fixed for version 4 and `KAWPOW_VERSION` in [#1452](https://github.com/KomodoPlatform/atomicDEX-API/pull/1452)
- Error messages for failing swaps due to a time difference between maker and taker are now more informative after [#1677](https://github.com/KomodoPlatform/atomicDEX-API/pull/1677)
- Fix `LBC` block header deserialization bug [#1343](https://github.com/KomodoPlatform/atomicDEX-API/pull/1343)
- Fix `NMC` block header deserialization bug [#1409](https://github.com/KomodoPlatform/atomicDEX-API/pull/1409)
- Refactor mm2 error handling for some structures [#1444](https://github.com/KomodoPlatform/atomicDEX-API/pull/1444)
- Tx wait for confirmation timeout fix [#1446](https://github.com/KomodoPlatform/atomicDEX-API/pull/1446)
- Retry tx wait confirmation if not on chain [#1474](https://github.com/KomodoPlatform/atomicDEX-API/pull/1474)
- Fix electrum "response is too large (over 2M bytes)" error for block header download [#1506](https://github.com/KomodoPlatform/atomicDEX-API/pull/1506)
- Deactivate tokens with platform coin [#1525](https://github.com/KomodoPlatform/atomicDEX-API/pull/1525)
- Enhanced logging in` spv` and `rpc_client` mods [#1594](https://github.com/KomodoPlatform/atomicDEX-API/pull/1594)
- Update metrics related dep && refactoring [#1312](https://github.com/KomodoPlatform/atomicDEX-API/pull/1312)
- Fix rick and morty genesis block deserialization [#1647](https://github.com/KomodoPlatform/atomicDEX-API/pull/1647)
- In `librustzcash` bumped `bech32` to `0.9.1`(which we already have in mm2, so we will not have 2 versions of `bech32`)
- Use dev branch as a target branch for Dependabot [#1424](https://github.com/KomodoPlatform/atomicDEX-API/pull/1424)
- Fixed Zhtlc orders is_mine bug (orders had "is_mine":false)  [#1489](https://github.com/KomodoPlatform/atomicDEX-API/pull/1489)
- Grouped SwapOps method arguments into new groups(structures) [#1529](https://github.com/KomodoPlatform/atomicDEX-API/pull/1529)
- Handling multiple rpcs optimization [#1480](https://github.com/KomodoPlatform/atomicDEX-API/issues/1480)
  - Tendermint multiple rpcs optimization [#1568](https://github.com/KomodoPlatform/atomicDEX-API/pull/1568)
  - Multiple rpcs optimization for `z_rpc` and `http_transport` [#1653](https://github.com/KomodoPlatform/atomicDEX-API/pull/1653)
  - Refactor p2p message processing flow (related with one of the security problem) [#1436](https://github.com/KomodoPlatform/atomicDEX-API/pull/1436)
- Solana tests are disabled [#1660](https://github.com/KomodoPlatform/atomicDEX-API/pull/1660)
- Some of vulnerable dependencies(tokio, libp2p) are fixed [#1666](https://github.com/KomodoPlatform/atomicDEX-API/pull/1666)
- Add `mm2_stop` WASM FFI [#1628](https://github.com/KomodoPlatform/atomicDEX-API/pull/1628)
- Use `futures_timer` crate and fix some unstable tests [#1511](https://github.com/KomodoPlatform/atomicDEX-API/pull/1511)
- Fix `Timer::sleep_ms` in WASM [#1514](https://github.com/KomodoPlatform/atomicDEX-API/pull/1514)
- Fix a race condition in `AbortableQueue` [#1528](https://github.com/KomodoPlatform/atomicDEX-API/pull/1528)
- Spawn `process_json_request` so the RPC requests can be processed asynchronously [#1620](https://github.com/KomodoPlatform/atomicDEX-API/pull/1620)
- Fix `task::-::cancel` if the RPC task is an awaiting status [#1582](https://github.com/KomodoPlatform/atomicDEX-API/pull/1582)
- `disable_coin` should fail if there are tokens dependent on the platform [#1651](https://github.com/KomodoPlatform/atomicDEX-API/pull/1651)
- Implement a repeatable future [#1564](https://github.com/KomodoPlatform/atomicDEX-API/pull/1564)
- Version handling was enhanced [#1686](https://github.com/KomodoPlatform/atomicDEX-API/pull/1686)
  - Version of `mm2_bin_lib` from cargo manifest is now used for the API version
  - `--version`, `-v`, `version` arguments now print the mm2 version
- Workflow for VirusTotal results was added to CI [#1676](https://github.com/KomodoPlatform/atomicDEX-API/pull/1676)
- `parity-ethereum` and `testcontainers-rs` crates from KomodoPlatform repo are now used [#1690](https://github.com/KomodoPlatform/atomicDEX-API/pull/1690)
- Testnet node of atom was updated, RUSTSEC-2023-0018 was ignored [#1692](https://github.com/KomodoPlatform/atomicDEX-API/pull/1692)
- Timestamp value sent from the peer in `PubkeyKeepAlive` msg was ignored and the received timestamp was used instead [#1668](https://github.com/KomodoPlatform/atomicDEX-API/pull/1668)
- Change release branch from mm2.1 to main in CI [#1697](https://github.com/KomodoPlatform/atomicDEX-API/pull/1697)
- CHANGELOG.md was introduced to have a complete log of code changes [#1680](https://github.com/KomodoPlatform/atomicDEX-API/pull/1680)
- Small fixes [#1518](https://github.com/KomodoPlatform/atomicDEX-API/pull/1518), [#1515](https://github.com/KomodoPlatform/atomicDEX-API/pull/1515), [#1550](https://github.com/KomodoPlatform/atomicDEX-API/pull/1657), [#1657](https://github.com/KomodoPlatform/atomicDEX-API/pull/1657)

**NB - Backwards compatibility breaking changes:**
- Because of [#1548](https://github.com/KomodoPlatform/atomicDEX-API/pull/1548), old nodes will not be able to swap BTC segwit with new nodes since locktimes are exchanged and validated in the negotiation messages.
