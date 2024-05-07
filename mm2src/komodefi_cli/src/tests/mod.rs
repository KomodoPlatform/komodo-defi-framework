use std::io::Write;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};

use crate::activation_scheme_db::{get_activation_scheme, get_activation_scheme_path, init_activation_scheme};
use crate::cli::Cli;
use crate::config::KomodefiConfigImpl;
use crate::komodefi_proc::ResponseHandlerImpl;
use crate::rpc_data::activation::{ActivationMethod, ActivationMethodLegacy};

const FAKE_SERVER_COOLDOWN_TIMEOUT_MS: u64 = 10;
const FAKE_SERVER_WARMUP_TIMEOUT_MS: u64 = 100;

#[tokio::test]
async fn test_get_version() {
    let mut buffer: Vec<u8> = vec![];
    let mock_resp = include_bytes!("http_mock_data/version.http");
    let command = &["komodefi-cli", "mm2", "version"];
    start_server_and_do_command(command, &mut buffer, mock_resp, 7784).await;
    assert_eq!(VERSION_OUTPUT, String::from_utf8(buffer).unwrap());
}

#[tokio::test]
async fn test_get_orderbook() {
    let mut buffer: Vec<u8> = vec![];
    let mock_resp = include_bytes!("http_mock_data/orderbook.http");
    let command = &["komodefi-cli", "orders", "book", "RICK", "MORTY"];
    start_server_and_do_command(command, &mut buffer, mock_resp, 7785).await;
    assert_eq!(RICK_AND_MORTY_ORDERBOOK, String::from_utf8(buffer).unwrap());
}

#[tokio::test]
async fn test_get_orderbook_with_uuids() {
    let mut buffer: Vec<u8> = vec![];
    let mock_resp = include_bytes!("http_mock_data/orderbook.http");
    let command = &["komodefi-cli", "orders", "book", "RICK", "MORTY", "--uuids"];
    start_server_and_do_command(command, &mut buffer, mock_resp, 7786).await;
    assert_eq!(RICK_AND_MORTY_ORDERBOOK_WITH_UUIDS, String::from_utf8(buffer).unwrap());
}

#[tokio::test]
async fn test_get_orderbook_with_publics() {
    let mut buffer: Vec<u8> = vec![];
    let mock_resp = include_bytes!("http_mock_data/orderbook.http");
    let command = &["komodefi-cli", "orders", "book", "RICK", "MORTY", "--publics"];
    start_server_and_do_command(command, &mut buffer, mock_resp, 7787).await;
    assert_eq!(ORDERBOOK_WITH_PUBLICS, String::from_utf8(buffer).unwrap());
}

#[tokio::test]
async fn test_get_enabled() {
    let mut buffer: Vec<u8> = vec![];
    let mock_resp = include_bytes!("http_mock_data/get_enabled.http");
    let command = &["komodefi-cli", "coin", "get-enabled"];
    start_server_and_do_command(command, &mut buffer, mock_resp, 7788).await;
    assert_eq!(ENABLED_COINS, String::from_utf8(buffer).unwrap());
}

#[tokio::test]
async fn test_get_balance() {
    let mut buffer: Vec<u8> = vec![];
    let mock_resp = include_bytes!("http_mock_data/balance.http");
    let command = &["komodefi-cli", "wallet", "balance", "RICK"];
    start_server_and_do_command(command, &mut buffer, mock_resp, 7789).await;
    assert_eq!(RICK_BALANCE, String::from_utf8(buffer).unwrap());
}

#[tokio::test]
async fn test_enable() {
    test_activation_scheme().await;
    let mut buffer: Vec<u8> = vec![];
    let mock_resp = include_bytes!("http_mock_data/enable.http");
    let command = &["komodefi-cli", "coin", "enable", "ETH"];
    start_server_and_do_command(command, &mut buffer, mock_resp, 7790).await;
    assert_eq!(ENABLE_OUTPUT, String::from_utf8(buffer).unwrap());
}

async fn test_activation_scheme() {
    let _ = std::fs::remove_file(get_activation_scheme_path().unwrap());
    init_activation_scheme().await.unwrap();
    let scheme = get_activation_scheme().unwrap();
    let kmd_scheme = scheme.get_activation_method("KMD");
    let Ok(ActivationMethod::Legacy(ActivationMethodLegacy::Electrum(electrum))) = kmd_scheme else {
         panic!("Failed to get electrum scheme")
    };
    assert_ne!(electrum.servers.len(), 0);
}

#[tokio::test]
async fn test_buy_morty_for_rick() {
    let mut buffer: Vec<u8> = vec![];
    let mock_resp = include_bytes!("http_mock_data/buy.http");
    let command = &["komodefi-cli", "buy", "MORTY", "RICK", "0.01", "0.5"];
    start_server_and_do_command(command, &mut buffer, mock_resp, 7791).await;
    assert_eq!(
        "4685e133-dfb3-4b31-8d4c-0ffa79933c8e\n",
        String::from_utf8(buffer).unwrap()
    );
}

#[tokio::test]
async fn test_order_status() {
    let mut buffer: Vec<u8> = vec![];
    let mock_resp = include_bytes!("http_mock_data/taker_status.http");
    let command = &[
        "komodefi-cli",
        "order",
        "status",
        "b7611502-eae8-4855-8bd7-16d992f952bf",
    ];
    start_server_and_do_command(command, &mut buffer, mock_resp, 7792).await;
    assert_eq!(TAKER_STATUS_OUTPUT, String::from_utf8(buffer).unwrap());
}

#[tokio::test]
async fn test_my_orders() {
    let mut buffer: Vec<u8> = vec![];
    let mock_resp = include_bytes!("http_mock_data/my_orders.http");
    let command = &["komodefi-cli", "orders", "mine"];
    start_server_and_do_command(command, &mut buffer, mock_resp, 7793).await;
    assert_eq!(MY_ORDERS_OUTPUT, String::from_utf8(buffer).unwrap());
}

#[tokio::test]
async fn test_best_orders() {
    let mut buffer: Vec<u8> = vec![];
    let mock_resp = include_bytes!("http_mock_data/best_orders.http");
    let command = &["komodefi-cli", "orders", "best", "--number", "2", "RICK", "buy"];
    start_server_and_do_command(command, &mut buffer, mock_resp, 7794).await;
    assert_eq!(BEST_ORDERS_OUTPUT, String::from_utf8(buffer).unwrap());
}

#[tokio::test]
async fn test_orderbook_depth() {
    let mut buffer: Vec<u8> = vec![];
    let mock_resp = include_bytes!("http_mock_data/orderbook_depth.http");
    let command = &["komodefi-cli", "orders", "depth", "RICK/MORTY", "BTC/KMD", "BTC/ETH"];
    start_server_and_do_command(command, &mut buffer, mock_resp, 7795).await;
    assert_eq!(ORDERBOOK_DEPTH_OUTPUT, String::from_utf8(buffer).unwrap());
}

#[tokio::test]
async fn test_history_common() {
    let mut buffer: Vec<u8> = vec![];
    let mock_resp = include_bytes!("http_mock_data/history-common.http");
    let command = &["komodefi-cli", "orders", "history", "--all"];
    start_server_and_do_command(command, &mut buffer, mock_resp, 7796).await;
    assert_eq!(HISTORY_COMMON_OUTPUT, String::from_utf8(buffer).unwrap());
}

#[tokio::test]
async fn test_history_takers_detailed() {
    let mut buffer: Vec<u8> = vec![];
    let mock_resp = include_bytes!("http_mock_data/history-takers-detailed.http");
    let command = &["komodefi-cli", "orders", "history", "--takers"];
    start_server_and_do_command(command, &mut buffer, mock_resp, 7797).await;
    assert_eq!(HISTORY_TAKERS_DETAILED_OUTPUT, String::from_utf8(buffer).unwrap());
}

#[tokio::test]
async fn test_history_makers_detailed() {
    let mut buffer: Vec<u8> = vec![];
    let mock_resp = include_bytes!("http_mock_data/history-makers-detailed.http");
    let command = &["komodefi-cli", "orders", "history", "--makers"];
    start_server_and_do_command(command, &mut buffer, mock_resp, 7798).await;
    assert_eq!(HISTORY_MAKERS_DETAILED_OUTPUT, String::from_utf8(buffer).unwrap());
}

#[tokio::test]
async fn test_active_swaps() {
    let mut buffer: Vec<u8> = vec![];
    let mock_resp = include_bytes!("http_mock_data/active_swaps.http");
    let command = &["komodefi-cli", "swaps", "active", "--include-status"];
    start_server_and_do_command(command, &mut buffer, mock_resp, 7799).await;
    assert_eq!(ACTIVE_SWAPS_OUTPUT, String::from_utf8(buffer).unwrap());
}

async fn fake_mm2_server(port: u16, predefined_response: &'static [u8]) {
    let server = TcpListener::bind(("0.0.0.0", port))
        .await
        .expect("Failed to bind tcp server");

    if let Ok((stream, _)) = server.accept().await {
        tokio::spawn(handle_connection(stream, predefined_response));
    }
}

async fn handle_connection(mut stream: TcpStream, predefined_response: &'static [u8]) {
    let (reader, mut writer) = stream.split();
    reader.readable().await.unwrap();
    writer.write_all(predefined_response).await.unwrap();
    tokio::time::sleep(Duration::from_millis(FAKE_SERVER_COOLDOWN_TIMEOUT_MS)).await;
}

async fn start_server_and_do_command(args: &[&str], buf: &mut dyn Write, mock_resp: &'static [u8], port: u16) {
    tokio::spawn(fake_mm2_server(port, mock_resp));
    tokio::time::sleep(Duration::from_millis(FAKE_SERVER_WARMUP_TIMEOUT_MS)).await;

    let response_handler = ResponseHandlerImpl { writer: buf.into() };
    let config = KomodefiConfigImpl::new("dummy", format!("http://127.0.0.1:{port}").as_str());
    Cli::execute(args.iter().map(|arg| arg.to_string()), &config, &response_handler)
        .await
        .unwrap();
}

const VERSION_OUTPUT: &str = "\
Version: 1.0.1-beta_824ca36f3
Datetime: 2023-04-06T22:35:43+05:00
";

const RICK_AND_MORTY_ORDERBOOK: &str = r"     Volume: RICK Price: MORTY     
             0.23 1.00000000       
        340654.03 1.00000000       
             2.00 0.99999999       
             2.00 0.99999999       
             2.00 0.99999999       
- --------------- ---------------- 
             0.96 1.02438024       
             1.99 1.00000001       
             1.99 1.00000001       
             1.99 1.00000001       
         32229.14 1.00000000       
             0.22 1.00000000       
";

const RICK_AND_MORTY_ORDERBOOK_WITH_UUIDS: &str = r"     Volume: RICK Price: MORTY     Uuid                                 
             0.23 1.00000000       c7585a1b-6060-4319-9da6-c67321628a06 
        340654.03 1.00000000       d69fe2a9-51ca-4d69-96ad-b141a01d8bb4 
             2.00 0.99999999       a2337218-7f6f-46a1-892e-6febfb7f5403 
             2.00 0.99999999       c172c295-7fe3-4131-9c81-c3a7182f0617 
             2.00 0.99999999       fbbc44d2-fb50-4b4b-8ac3-d9857cae16b6 
- --------------- ---------------- ------------------------------------ 
             0.96 1.02438024       c480675b-3352-4159-9b3c-55cb2b1329de 
             1.99 1.00000001       fdb0de9c-e283-48c3-9de6-8117fecf0aff 
             1.99 1.00000001       6a3bb75d-8e91-4192-bf50-d8190a69600d 
             1.99 1.00000001       b24b40de-e93d-4218-8d93-1940ceadce7f 
         32229.14 1.00000000       652a7e97-f42c-4f87-bc26-26bd1a0fea24 
             0.22 1.00000000       1082c93c-8c23-4944-b8f1-a92ec703b03a 
";

const ORDERBOOK_WITH_PUBLICS: &str = r"     Volume: RICK Price: MORTY     Public                                                             
             0.23 1.00000000       022d7424c741213a2b9b49aebdaa10e84419e642a8db0a09e359a3d4c850834846 
        340654.03 1.00000000       0315d9c51c657ab1be4ae9d3ab6e76a619d3bccfe830d5363fa168424c0d044732 
             2.00 0.99999999       037310a8fb9fd8f198a1a21db830252ad681fccda580ed4101f3f6bfb98b34fab5 
             2.00 0.99999999       037310a8fb9fd8f198a1a21db830252ad681fccda580ed4101f3f6bfb98b34fab5 
             2.00 0.99999999       037310a8fb9fd8f198a1a21db830252ad681fccda580ed4101f3f6bfb98b34fab5 
- --------------- ---------------- ------------------------------------------------------------------ 
             0.96 1.02438024       02d6c3e22a419a4034272acb215f1d39cd6a0413cfd83ac0c68f482db80accd89a 
             1.99 1.00000001       037310a8fb9fd8f198a1a21db830252ad681fccda580ed4101f3f6bfb98b34fab5 
             1.99 1.00000001       037310a8fb9fd8f198a1a21db830252ad681fccda580ed4101f3f6bfb98b34fab5 
             1.99 1.00000001       037310a8fb9fd8f198a1a21db830252ad681fccda580ed4101f3f6bfb98b34fab5 
         32229.14 1.00000000       0315d9c51c657ab1be4ae9d3ab6e76a619d3bccfe830d5363fa168424c0d044732 
             0.22 1.00000000       022d7424c741213a2b9b49aebdaa10e84419e642a8db0a09e359a3d4c850834846 
";

const ENABLED_COINS: &str = "\
Ticker   Address
MORTY    RPFGrvJWjSYN4qYvcXsECW1HoHbvQjowZM
RICK     RPFGrvJWjSYN4qYvcXsECW1HoHbvQjowZM
KMD      RPFGrvJWjSYN4qYvcXsECW1HoHbvQjowZM
ETH      0x224050fb7EB13Fa0D342F5b245f1237bAB531650
";

const RICK_BALANCE: &str = "\
coin: RICK
balance: 0.5767226
unspendable: 0
address: RPFGrvJWjSYN4qYvcXsECW1HoHbvQjowZM
";

const ENABLE_OUTPUT: &str = "\
coin: ETH
address: 0x224050fb7EB13Fa0D342F5b245f1237bAB531650
balance: 0.02
unspendable_balance: 0
required_confirmations: 3
requires_notarization: No
";

const TAKER_STATUS_OUTPUT: &str = r"                uuid: 1ae94a08-47e3-4938-bebb-5df8ff74b8e0
      req.(base,rel): MORTY(0.0100), RICK(0.01)
          req.action: Buy
  req.(sender, dest): 264fcd9401d797c50fe2f1c7d5fe09bbc10f3838c1d8d6f793061fa5f38b2b4d, 0000000000000000000000000000000000000000000000000000000000000000
        req.match_by: Any
   req.conf_settings: 111,true:555,true
          created_at: 23-05-11 19:28:46
          order_type: GoodTillCancelled
         cancellable: false
             matches: 
                      uuid: 600f62b3-5248-4905-9618-14f339cc7d30
       reserved.(base,rel): MORTY(0.0100), RICK(0.0099)
   reserved.(taker, maker): 1ae94a08-47e3-4938-bebb-5df8ff74b8e0,600f62b3-5248-4905-9618-14f339cc7d30
   reserved.(sender, dest): 7310a8fb9fd8f198a1a21db830252ad681fccda580ed4101f3f6bfb98b34fab5,0000000000000000000000000000000000000000000000000000000000000000
    reserved.conf_settings: 1,false:1,false
              last_updated: none
     connect.(taker,maker): 1ae94a08-47e3-4938-bebb-5df8ff74b8e0,600f62b3-5248-4905-9618-14f339cc7d30
    connect.(sender, dest): 264fcd9401d797c50fe2f1c7d5fe09bbc10f3838c1d8d6f793061fa5f38b2b4d,7310a8fb9fd8f198a1a21db830252ad681fccda580ed4101f3f6bfb98b34fab5
";

const MY_ORDERS_OUTPUT: &str = "        Taker orders: 
│ action                   │ uuid, sender, dest                                               │ type,created_at          │ match_by                 │ base,rel                 │ cancellable               │
│ base(vol),rel(vol)       │                                                                  │ confirmation             │                          │ orderbook ticker         │                           │
│ Buy                      │ 2739152a-3f87-4f6d-a199-3659aa1e864f                             │ GoodTillCancelled        │ Any                      │ none                     │ true                      │
│ MORTY(0.10),RICK(0.09)   │ 264fcd9401d797c50fe2f1c7d5fe09bbc10f3838c1d8d6f793061fa5f38b2b4d │ 23-05-29 12:18:52        │                          │ none                     │                           │
│                          │ 0000000000000000000000000000000000000000000000000000000000000000 │ 1,false:1,false          │                          │                          │                           │
│ Buy                      │ ce90f89f-8074-4e9f-8649-7f7689c56fa9                             │ GoodTillCancelled        │ Any                      │ none                     │ false                     │
│ MORTY(0.10),RICK(0.11)   │ 264fcd9401d797c50fe2f1c7d5fe09bbc10f3838c1d8d6f793061fa5f38b2b4d │ 23-05-29 12:19:10        │                          │ none                     │                           │
│                          │ 0000000000000000000000000000000000000000000000000000000000000000 │ 1,false:1,false          │                          │                          │                           │
│ matches                                                                                                                                                                                                  │
│                       uuid: 09a0e11e-837e-4763-bc1f-1659573df9dd                                                                                                                                         │
│        reserved.(base,rel): MORTY(0.10), RICK(0.09)                                                                                                                                                      │
│    reserved.(taker, maker): ce90f89f-8074-4e9f-8649-7f7689c56fa9,09a0e11e-837e-4763-bc1f-1659573df9dd                                                                                                    │
│    reserved.(sender, dest): 7310a8fb9fd8f198a1a21db830252ad681fccda580ed4101f3f6bfb98b34fab5,0000000000000000000000000000000000000000000000000000000000000000                                            │
│     reserved.conf_settings: 1,false:1,false                                                                                                                                                              │
│               last_updated: none                                                                                                                                                                         │
│      connect.(taker,maker): ce90f89f-8074-4e9f-8649-7f7689c56fa9,09a0e11e-837e-4763-bc1f-1659573df9dd                                                                                                    │
│     connect.(sender, dest): 264fcd9401d797c50fe2f1c7d5fe09bbc10f3838c1d8d6f793061fa5f38b2b4d,7310a8fb9fd8f198a1a21db830252ad681fccda580ed4101f3f6bfb98b34fab5                                            │
│                                                                                                                                                                                                          │

        Maker orders: 
│ base,rel   │ price │ uuid                                 │ created at,        │ min base vol, │ cancellable │ available │ swaps │ conf_settings   │ history changes │
│            │       │                                      │ updated at         │ max base vol  │             │ amount    │       │                 │                 │
│ RICK,MORTY │ 1.11  │ 28315c31-4fd7-4847-9873-352924252fbe │ 23-05-29 12:17:46, │ 0.000100,     │ true        │ 0.09      │ empty │ 1,false:1,false │ none            │
│            │       │                                      │ 23-05-29 12:17:46  │ 0.09          │             │           │       │                 │                 │
│ RICK,MORTY │ 1.11  │ 7f097435-f482-415b-9bdf-6780f4be4828 │ 23-05-29 12:17:49, │ 0.000100,     │ true        │ 0.09      │ empty │ 1,false:1,false │ none            │
│            │       │                                      │ 23-05-29 12:17:49  │ 0.09          │             │           │       │                 │                 │

";

const BEST_ORDERS_OUTPUT:&str = "\
│  │ Price  │ Uuid                                 │ Base vol(min:max)  │ Rel vol(min:max)   │ Address                            │ Confirmation    │
│ KMD                                                                                                                                               │
│  │ 0.0050 │ 7c643319-52ea-4323-b0d2-1c448cfc007d │ 0.02:9730.65       │ 0.00010:48.65      │ REbPB4qfrB2D5KAnJJK1RTC1CLGa8hVEcM │ 1,false:2,true  │
│ MORTY                                                                                                                                             │
│  │ 1.00   │ 2af2d0f3-35e8-4098-8362-99ec9867b9ac │ 0.000100:363783.58 │ 0.000100:363783.58 │ RB8yufv3YTfdzYnwz5paNnnDynGJG6WsqD │ 1,false:1,false │
│  │ 0.99   │ e52246a2-f9b2-4145-9aa6-53b96bfabe9f │ 0.00010:2.00       │ 0.000100:1.99      │ RMaprYNUp8ErJ9ZAKcxMfpC4ioVycYCCCc │ 1,false:1,false │
│ ZOMBIE                                                                                                                                            │
│  │ 1.00   │ 2536e0d8-0a8b-4393-913b-d74543733e5e │ 0.000100:0.23      │ 0.000100:0.23      │ Shielded                           │ 1,false:1,false │
";

const ORDERBOOK_DEPTH_OUTPUT: &str = "             Bids Asks 
    BTC/KMD: 5    1    
    BTC/ETH: 0    1    
 RICK/MORTY: 5    5    
";

const HISTORY_COMMON_OUTPUT: &str = "\
Orders history:
│uuid                                │Type │Action│Base│Rel  │Volume │Price  │Status   │Created          │Updated          │Was taker│
│                                    │     │      │    │     │       │       │         │                 │                 │         │
│010a224e-a946-4726-bf6d-e521701053a2│Maker│Sell  │RICK│MORTY│8.16   │1.23   │Cancelled│23-06-06 15:41:33│23-06-07 10:37:47│false    │
│ffc41a51-e110-43a0-bb60-203feceba50f│Maker│Sell  │RICK│MORTY│8.16   │1.23   │Cancelled│23-06-06 15:41:17│23-06-06 15:41:33│false    │
│869cd8d1-914d-4756-a863-6f73e004c31c│Maker│Sell  │RICK│MORTY│8.16   │1.23   │Cancelled│23-06-06 15:38:36│23-06-06 15:41:33│false    │
│3af195fe-f202-428d-8849-6c0b7754e894│Maker│Sell  │RICK│MORTY│8.16   │1.23   │Cancelled│23-06-06 15:38:30│23-06-06 15:38:36│false    │
│73271a03-aab3-4789-83d9-9e9c3c808a96│Maker│Sell  │RICK│MORTY│8.16   │1.23   │Cancelled│23-06-06 15:38:04│23-06-06 15:38:30│false    │
│e3be3027-333a-4867-928d-61e8442db466│Maker│Sell  │RICK│MORTY│8.16   │1.23   │Cancelled│23-06-06 15:37:49│23-06-06 15:38:04│false    │
│a7a04dc8-c361-4cae-80e9-b0e883aa3ae1│Maker│Sell  │RICK│MORTY│8.16   │1.23   │Cancelled│23-06-06 15:36:49│23-06-06 15:37:49│false    │
│ecc708e0-df8f-4d3f-95c7-73927ec92acc│Maker│Sell  │RICK│MORTY│8.16   │1.23   │Cancelled│23-06-06 15:35:47│23-06-06 15:36:48│false    │
│e1797608-5b7d-45c4-80ae-b66da2e72209│Maker│Sell  │RICK│MORTY│8.16   │1.23   │Cancelled│23-06-06 15:35:16│23-06-06 15:35:47│false    │
│f164e567-9e41-4faf-8754-3f87edd5b6d7│Maker│Sell  │RICK│MORTY│8.16   │1.23   │Cancelled│23-06-06 15:32:32│23-06-06 15:35:16│false    │
│707c8428-779c-4e78-bcbd-97a7e403c14a│Maker│Sell  │RICK│MORTY│8.16   │1.23   │Cancelled│23-06-06 15:31:54│23-06-06 15:32:32│false    │
│b0992fe8-c019-4c86-9d07-03055eaa86ab│Maker│Sell  │RICK│MORTY│2.00   │1.50   │Cancelled│23-06-06 15:23:34│23-06-06 15:31:54│false    │
│85d6fc7c-5614-492a-9e85-4c19fab65949│Maker│Sell  │RICK│MORTY│2.00   │1.50   │Cancelled│23-06-06 15:23:07│23-06-06 15:23:20│false    │
│5968ffcf-5b25-40c8-8bd7-c7cf9d3154f9│Maker│Sell  │RICK│MORTY│8.16   │1.50   │Cancelled│23-06-06 15:22:51│23-06-06 15:23:07│false    │
│eab52e14-1460-4ece-943d-7a2950a22600│Maker│Sell  │RICK│MORTY│2.00   │1.50   │Cancelled│23-06-06 15:21:31│23-06-06 15:21:59│false    │
│4318bf91-8416-417d-ac30-7745f30df687│Maker│Sell  │RICK│MORTY│2.00   │1000.00│Cancelled│23-06-06 15:21:17│23-06-06 15:21:31│false    │
│a2f6930d-b97d-4c8c-9330-54912fd3b0b9│Maker│Sell  │RICK│MORTY│8.16   │1000.00│Cancelled│23-06-06 15:20:55│23-06-06 15:21:10│false    │
│d68a81fd-7a90-4785-ad83-d3b06e362f6f│Maker│Sell  │RICK│MORTY│0.00100│1000.00│Cancelled│23-06-06 15:18:05│23-06-06 15:20:55│false    │
│4c0ca34a-487c-43ef-b1f5-13eb4e1a8ece│Maker│Sell  │RICK│MORTY│0.00100│1.10   │Cancelled│23-06-06 15:17:45│23-06-06 15:18:05│false    │
│cba44f7f-5d52-492e-a3f0-44ee006296da│Maker│Sell  │RICK│MORTY│1.50   │1.10   │Cancelled│23-06-06 15:13:57│23-06-06 15:17:45│false    │
│02db133a-5e69-4056-9855-98d961927fdd│Maker│Sell  │RICK│MORTY│1.50   │1.10   │Cancelled│23-06-06 15:09:17│23-06-06 15:13:57│false    │
│6476641f-9014-496c-a608-1bdf81cfbf2e│Maker│Sell  │RICK│MORTY│8.16   │1.10   │Cancelled│23-06-06 15:08:58│23-06-06 15:09:17│false    │
│5a253d33-7c7c-40f5-977f-7805013e63b4│Maker│Sell  │RICK│MORTY│8.16   │1.10   │Cancelled│23-06-06 15:06:17│23-06-06 15:06:28│false    │
│064bf73f-2a2a-4ca0-b83f-344ec16c5f29│Maker│Sell  │RICK│MORTY│8.16   │1.20   │Cancelled│23-06-06 15:04:52│23-06-06 15:06:17│false    │
│475309b5-d6e1-40b2-a2d4-5307aa999d74│Maker│Sell  │RICK│MORTY│1.33   │1.20   │Cancelled│23-06-06 15:04:33│23-06-06 15:04:52│false    │
│916bbc09-6b57-4ded-93b0-5a8461be0e99│Maker│Sell  │RICK│MORTY│0.50   │1.20   │Cancelled│23-06-06 14:53:06│23-06-06 15:03:07│false    │
│fa256795-9ff3-4983-85d6-8a3fe4fb6f3a│Maker│Sell  │RICK│MORTY│8.16   │1.20   │Cancelled│23-06-06 14:52:20│23-06-06 14:52:59│false    │
│23d2c04b-6fa5-4e76-bde9-4a8fe0b7a144│Maker│Sell  │RICK│MORTY│8.16   │1.10   │Cancelled│23-06-06 14:51:40│23-06-06 14:52:20│false    │
│4e365431-4db0-4365-a67d-1e39820090a2│Taker│Buy   │RICK│MORTY│0.05   │1.10   │TimedOut │23-05-05 14:35:31│23-05-05 14:36:02│false    │
│601bfc00-9033-45d8-86b2-3dbd54881212│Taker│Buy   │RICK│MORTY│0.05   │1.10   │Fulfilled│23-05-05 14:34:55│23-05-05 14:34:58│false    │
";

const HISTORY_TAKERS_DETAILED_OUTPUT: &str = "\
Taker orders history detailed:
│ action                   │ uuid, sender, dest                                               │ type,created_at          │ match_by                 │ base,rel                 │ cancellable               │
│ base(vol),rel(vol)       │                                                                  │ confirmation             │                          │ orderbook ticker         │                           │
│                          │                                                                  │                          │                          │                          │                           │
│ Buy                      │ 4e365431-4db0-4365-a67d-1e39820090a2                             │ GoodTillCancelled        │ Any                      │ none                     │ false                     │
│ RICK(0.05),MORTY(0.05)   │ 264fcd9401d797c50fe2f1c7d5fe09bbc10f3838c1d8d6f793061fa5f38b2b4d │ 23-05-05 14:35:31        │                          │ none                     │                           │
│                          │ 0000000000000000000000000000000000000000000000000000000000000000 │ 1,false:1,false          │                          │                          │                           │
│ matches                                                                                                                                                                                                  │
│                       uuid: efbcb9d6-2d9d-4fa0-af82-919c7da46967                                                                                                                                         │
│        reserved.(base,rel): RICK(0.05), MORTY(0.04)                                                                                                                                                      │
│    reserved.(taker, maker): 4e365431-4db0-4365-a67d-1e39820090a2,efbcb9d6-2d9d-4fa0-af82-919c7da46967                                                                                                    │
│    reserved.(sender, dest): 7310a8fb9fd8f198a1a21db830252ad681fccda580ed4101f3f6bfb98b34fab5,0000000000000000000000000000000000000000000000000000000000000000                                            │
│     reserved.conf_settings: 0,false:0,false                                                                                                                                                              │
│               last_updated: 23-05-05 14:35:34                                                                                                                                                            │
│      connect.(taker,maker): 4e365431-4db0-4365-a67d-1e39820090a2,efbcb9d6-2d9d-4fa0-af82-919c7da46967                                                                                                    │
│     connect.(sender, dest): 264fcd9401d797c50fe2f1c7d5fe09bbc10f3838c1d8d6f793061fa5f38b2b4d,7310a8fb9fd8f198a1a21db830252ad681fccda580ed4101f3f6bfb98b34fab5                                            │
│                                                                                                                                                                                                          │
│ Buy                      │ 601bfc00-9033-45d8-86b2-3dbd54881212                             │ GoodTillCancelled        │ Any                      │ none                     │ false                     │
│ RICK(0.05),MORTY(0.05)   │ 264fcd9401d797c50fe2f1c7d5fe09bbc10f3838c1d8d6f793061fa5f38b2b4d │ 23-05-05 14:34:55        │                          │ none                     │                           │
│                          │ 0000000000000000000000000000000000000000000000000000000000000000 │ 1,false:1,false          │                          │                          │                           │
│ matches                                                                                                                                                                                                  │
│                       uuid: e16ee590-0562-4fbe-88cd-3cfd6e580615                                                                                                                                         │
│        reserved.(base,rel): RICK(0.05), MORTY(0.04)                                                                                                                                                      │
│    reserved.(taker, maker): 601bfc00-9033-45d8-86b2-3dbd54881212,e16ee590-0562-4fbe-88cd-3cfd6e580615                                                                                                    │
│    reserved.(sender, dest): 7310a8fb9fd8f198a1a21db830252ad681fccda580ed4101f3f6bfb98b34fab5,0000000000000000000000000000000000000000000000000000000000000000                                            │
│     reserved.conf_settings: 0,false:0,false                                                                                                                                                              │
│               last_updated: 23-05-05 14:34:58                                                                                                                                                            │
│      connect.(taker,maker): 601bfc00-9033-45d8-86b2-3dbd54881212,e16ee590-0562-4fbe-88cd-3cfd6e580615                                                                                                    │
│     connect.(sender, dest): 264fcd9401d797c50fe2f1c7d5fe09bbc10f3838c1d8d6f793061fa5f38b2b4d,7310a8fb9fd8f198a1a21db830252ad681fccda580ed4101f3f6bfb98b34fab5                                            │
│                                                                                                                                                                                                          │
";

const HISTORY_MAKERS_DETAILED_OUTPUT: &str = "\
Maker orders history detailed:
│ base,rel   │ price   │ uuid                                 │ created at,        │ min base vol, │ swaps │ conf_settings   │ history changes │ orderbook ticker │
│            │         │                                      │ updated at         │ max base vol  │       │                 │                 │ base, rel        │
│            │         │                                      │                    │               │       │                 │                 │                  │
│ RICK,MORTY │ 1.23    │ 010a224e-a946-4726-bf6d-e521701053a2 │ 23-06-06 15:41:33, │ 1.00,         │ empty │ 3,true:1,false  │ none            │ none             │
│            │         │                                      │ 23-06-06 15:41:33  │ 8.16          │       │                 │                 │ none             │
│ RICK,MORTY │ 1.23    │ ffc41a51-e110-43a0-bb60-203feceba50f │ 23-06-06 15:41:17, │ 1.00,         │ empty │ 3,true:1,false  │ none            │ none             │
│            │         │                                      │ 23-06-06 15:41:17  │ 8.16          │       │                 │                 │ none             │
│ RICK,MORTY │ 1.23    │ 869cd8d1-914d-4756-a863-6f73e004c31c │ 23-06-06 15:38:36, │ 1.00,         │ empty │ 3,true:1,false  │ none            │ none             │
│            │         │                                      │ 23-06-06 15:38:36  │ 8.16          │       │                 │                 │ none             │
│ RICK,MORTY │ 1.23    │ 3af195fe-f202-428d-8849-6c0b7754e894 │ 23-06-06 15:38:30, │ 1.00,         │ empty │ 3,true:1,false  │ none            │ none             │
│            │         │                                      │ 23-06-06 15:38:30  │ 8.16          │       │                 │                 │ none             │
│ RICK,MORTY │ 1.23    │ 73271a03-aab3-4789-83d9-9e9c3c808a96 │ 23-06-06 15:38:04, │ 1.00,         │ empty │ 3,true:1,false  │ none            │ none             │
│            │         │                                      │ 23-06-06 15:38:04  │ 8.16          │       │                 │                 │ none             │
│ RICK,MORTY │ 1.23    │ e3be3027-333a-4867-928d-61e8442db466 │ 23-06-06 15:37:49, │ 1.00,         │ empty │ 3,true:1,false  │ none            │ none             │
│            │         │                                      │ 23-06-06 15:37:49  │ 8.16          │       │                 │                 │ none             │
│ RICK,MORTY │ 1.23    │ a7a04dc8-c361-4cae-80e9-b0e883aa3ae1 │ 23-06-06 15:36:49, │ 1.00,         │ empty │ 3,true:1,false  │ none            │ none             │
│            │         │                                      │ 23-06-06 15:36:49  │ 8.16          │       │                 │                 │ none             │
│ RICK,MORTY │ 1.23    │ ecc708e0-df8f-4d3f-95c7-73927ec92acc │ 23-06-06 15:35:47, │ 1.00,         │ empty │ 1,false:1,false │ none            │ none             │
│            │         │                                      │ 23-06-06 15:35:47  │ 8.16          │       │                 │                 │ none             │
│ RICK,MORTY │ 1.23    │ e1797608-5b7d-45c4-80ae-b66da2e72209 │ 23-06-06 15:35:16, │ 0.000100,     │ empty │ 1,false:1,false │ none            │ none             │
│            │         │                                      │ 23-06-06 15:35:16  │ 8.16          │       │                 │                 │ none             │
│ RICK,MORTY │ 1.23    │ f164e567-9e41-4faf-8754-3f87edd5b6d7 │ 23-06-06 15:32:32, │ 0.000100,     │ empty │ 1,false:1,false │ none            │ none             │
│            │         │                                      │ 23-06-06 15:32:32  │ 8.16          │       │                 │                 │ none             │
│ RICK,MORTY │ 1.23    │ 707c8428-779c-4e78-bcbd-97a7e403c14a │ 23-06-06 15:31:54, │ 0.000100,     │ empty │ 1,false:1,false │ none            │ none             │
│            │         │                                      │ 23-06-06 15:31:54  │ 8.16          │       │                 │                 │ none             │
│ RICK,MORTY │ 1.50    │ b0992fe8-c019-4c86-9d07-03055eaa86ab │ 23-06-06 15:23:34, │ 0.000100,     │ empty │ 1,false:1,false │ none            │ none             │
│            │         │                                      │ 23-06-06 15:23:34  │ 2.00          │       │                 │                 │ none             │
│ RICK,MORTY │ 1.50    │ 85d6fc7c-5614-492a-9e85-4c19fab65949 │ 23-06-06 15:23:07, │ 0.000100,     │ empty │ 1,false:1,false │ none            │ none             │
│            │         │                                      │ 23-06-06 15:23:07  │ 2.00          │       │                 │                 │ none             │
│ RICK,MORTY │ 1.50    │ 5968ffcf-5b25-40c8-8bd7-c7cf9d3154f9 │ 23-06-06 15:22:51, │ 0.000100,     │ empty │ 1,false:1,false │ none            │ none             │
│            │         │                                      │ 23-06-06 15:22:51  │ 8.16          │       │                 │                 │ none             │
│ RICK,MORTY │ 1.50    │ eab52e14-1460-4ece-943d-7a2950a22600 │ 23-06-06 15:21:31, │ 0.000100,     │ empty │ 1,false:1,false │ none            │ none             │
│            │         │                                      │ 23-06-06 15:21:31  │ 2.00          │       │                 │                 │ none             │
│ RICK,MORTY │ 1000.00 │ 4318bf91-8416-417d-ac30-7745f30df687 │ 23-06-06 15:21:17, │ 0.000100,     │ empty │ 1,false:1,false │ none            │ none             │
│            │         │                                      │ 23-06-06 15:21:17  │ 2.00          │       │                 │                 │ none             │
│ RICK,MORTY │ 1000.00 │ a2f6930d-b97d-4c8c-9330-54912fd3b0b9 │ 23-06-06 15:20:55, │ 0.000100,     │ empty │ 1,false:1,false │ none            │ none             │
│            │         │                                      │ 23-06-06 15:20:55  │ 8.16          │       │                 │                 │ none             │
│ RICK,MORTY │ 1000.00 │ d68a81fd-7a90-4785-ad83-d3b06e362f6f │ 23-06-06 15:18:05, │ 0.000100,     │ empty │ 1,false:1,false │ none            │ none             │
│            │         │                                      │ 23-06-06 15:18:05  │ 0.00100       │       │                 │                 │ none             │
│ RICK,MORTY │ 1.10    │ 4c0ca34a-487c-43ef-b1f5-13eb4e1a8ece │ 23-06-06 15:17:45, │ 0.000100,     │ empty │ 1,false:1,false │ none            │ none             │
│            │         │                                      │ 23-06-06 15:17:45  │ 0.00100       │       │                 │                 │ none             │
│ RICK,MORTY │ 1.10    │ cba44f7f-5d52-492e-a3f0-44ee006296da │ 23-06-06 15:13:57, │ 0.000100,     │ empty │ 1,false:1,false │ none            │ none             │
│            │         │                                      │ 23-06-06 15:13:57  │ 1.50          │       │                 │                 │ none             │
│ RICK,MORTY │ 1.10    │ 02db133a-5e69-4056-9855-98d961927fdd │ 23-06-06 15:09:17, │ 0.000100,     │ empty │ 1,false:1,false │ none            │ none             │
│            │         │                                      │ 23-06-06 15:09:17  │ 1.50          │       │                 │                 │ none             │
│ RICK,MORTY │ 1.10    │ 6476641f-9014-496c-a608-1bdf81cfbf2e │ 23-06-06 15:08:58, │ 0.000100,     │ empty │ 1,false:1,false │ none            │ none             │
│            │         │                                      │ 23-06-06 15:08:58  │ 8.16          │       │                 │                 │ none             │
│ RICK,MORTY │ 1.10    │ 5a253d33-7c7c-40f5-977f-7805013e63b4 │ 23-06-06 15:06:17, │ 0.000100,     │ empty │ 1,false:1,false │ none            │ none             │
│            │         │                                      │ 23-06-06 15:06:17  │ 8.16          │       │                 │                 │ none             │
│ RICK,MORTY │ 1.20    │ 064bf73f-2a2a-4ca0-b83f-344ec16c5f29 │ 23-06-06 15:04:52, │ 0.000100,     │ empty │ 1,false:1,false │ none            │ none             │
│            │         │                                      │ 23-06-06 15:04:52  │ 8.16          │       │                 │                 │ none             │
│ RICK,MORTY │ 1.20    │ 475309b5-d6e1-40b2-a2d4-5307aa999d74 │ 23-06-06 15:04:33, │ 0.000100,     │ empty │ 1,false:1,false │ none            │ none             │
│            │         │                                      │ 23-06-06 15:04:33  │ 1.33          │       │                 │                 │ none             │
│ RICK,MORTY │ 1.20    │ 916bbc09-6b57-4ded-93b0-5a8461be0e99 │ 23-06-06 14:53:06, │ 0.000100,     │ empty │ 1,false:1,false │ none            │ none             │
│            │         │                                      │ 23-06-06 14:53:06  │ 0.50          │       │                 │                 │ none             │
│ RICK,MORTY │ 1.20    │ fa256795-9ff3-4983-85d6-8a3fe4fb6f3a │ 23-06-06 14:52:20, │ 0.000100,     │ empty │ 1,false:1,false │ none            │ none             │
│            │         │                                      │ 23-06-06 14:52:20  │ 8.16          │       │                 │                 │ none             │
│ RICK,MORTY │ 1.10    │ 23d2c04b-6fa5-4e76-bde9-4a8fe0b7a144 │ 23-06-06 14:51:40, │ 0.000100,     │ empty │ 1,false:1,false │ none            │ none             │
│            │         │                                      │ 23-06-06 14:51:40  │ 8.16          │       │                 │                 │ none             │
";

const ACTIVE_SWAPS_OUTPUT: &str = "
TakerSwap: 6b007706-d6e1-4565-8655-9eeb128d00e2
my_order_uuid: 6b007706-d6e1-4565-8655-9eeb128d00e2
gui: adex-cli
mm_version: 1.0.6-beta_dabdaf33b
taker_coin: DOC
taker_amount: 1.00
maker_coin: MARTY
maker_amount: 1.00
events: 
│ Started                           │ uuid: 6b007706-d6e1-4565-8655-9eeb128d00e2                                                                             │
│ 23-07-25 12:20:07                 │ started_at: 23-07-25 12:20:07                                                                                          │
│                                   │ taker_coin: DOC                                                                                                        │
│                                   │ maker_coin: MARTY                                                                                                      │
│                                   │ maker: 2d7424c741213a2b9b49aebdaa10e84419e642a8db0a09e359a3d4c850834846                                                │
│                                   │ my_persistent_pub: 02264fcd9401d797c50fe2f1c7d5fe09bbc10f3838c1d8d6f793061fa5f38b2b4d                                  │
│                                   │ lock_duration: 7800                                                                                                    │
│                                   │ maker_amount: 1.00                                                                                                     │
│                                   │ taker_amount: 1.00                                                                                                     │
│                                   │ maker_payment_confirmations: 1                                                                                         │
│                                   │ maker_payment_requires_nota: false                                                                                     │
│                                   │ taker_payment_confirmations: 1                                                                                         │
│                                   │ taker_payment_requires_nota: false                                                                                     │
│                                   │ tacker_payment_lock: 23-07-25 14:30:07                                                                                 │
│                                   │ maker_payment_wait: 23-07-25 13:12:07                                                                                  │
│                                   │ maker_coin_start_block: 147860                                                                                         │
│                                   │ taker_coin_start_block: 133421                                                                                         │
│                                   │ fee_to_send_taker_fee: coin: DOC, amount: 0.00001, paid_from_trading_vol: false                                        │
│                                   │ taker_payment_trade_fee: coin: DOC, amount: 0.00001, paid_from_trading_vol: false                                      │
│                                   │ maker_payment_spend_trade_fee: coin: MARTY, amount: 0.00001, paid_from_trading_vol: true                               │
│                                   │ maker_coin_htlc_pubkey: 02264fcd9401d797c50fe2f1c7d5fe09bbc10f3838c1d8d6f793061fa5f38b2b4d                             │
│                                   │ taker_coin_htlc_pubkey: 02264fcd9401d797c50fe2f1c7d5fe09bbc10f3838c1d8d6f793061fa5f38b2b4d                             │
│                                   │                                                                                                                        │
│ Negotiated                        │ maker_payment_locktime: 23-07-25 16:40:06                                                                              │
│ 23-07-25 12:20:23                 │ maker_pubkey: 000000000000000000000000000000000000000000000000000000000000000000                                       │
│                                   │ secret_hash: a5cfc9787066562ba03c7538d024a88fd1a0fe12                                                                  │
│                                   │ maker_coin_htlc_pubkey: 022d7424c741213a2b9b49aebdaa10e84419e642a8db0a09e359a3d4c850834846                             │
│                                   │ taker_coin_htlc_pubkey: 022d7424c741213a2b9b49aebdaa10e84419e642a8db0a09e359a3d4c850834846                             │
│                                   │                                                                                                                        │
│ TakerFeeSent                      │ tx_hex: 0400008085202f890108742b73cfadf56dbc93d3fb8b33b54e5301869e2c950b200e67354b56e2d2ef010000006a473044022073605008 │
│ 23-07-25 12:20:23                 │ 9328c8ec984036b4a248ba3a130d58e9601da3358ffef3482d40927002204a8aa7b83560ee22792457465432bf4f098b38f41d0a483f68920eb63b │
│                                   │ 487d02012102264fcd9401d797c50fe2f1c7d5fe09bbc10f3838c1d8d6f793061fa5f38b2b4dffffffff02bcf60100000000001976a914ca1e0474 │
│                                   │ 5e8ca0c60d8c5881531d51bec470743f88ace0adaf08635600001976a9149934ebeaa56cb597c936a9ed8202d8d97a0a700388ac07bebf64000000 │
│                                   │ 000000000000000000000000                                                                                               │
│                                   │ tx_hash: c71f3793e976209674e2b00efb236c0fa8f0b1b552cb6cfe9068c6b731e570fd                                              │
│                                   │                                                                                                                        │
│ TakerPaymentInstructionsReceived  │ none                                                                                                                   │
│ 23-07-25 12:20:24                 │                                                                                                                        │
│                                   │                                                                                                                        │
│ MakerPaymentReceived              │ tx_hex: 0400008085202f890754448b50295dedd36f8de60aeaeeb56a5efa9d1a4464185e329c3aae9fd17673020000006a4730440220651b3753 │
│ 23-07-25 12:20:24                 │ 986a47026f36b082d870c3b2f7651684c0ed26637b64bfbc8722059302200ee7478e290327827daff8a2daf836e9446362169a4a8a4958f538c07f │
│                                   │ 2093180121022d7424c741213a2b9b49aebdaa10e84419e642a8db0a09e359a3d4c850834846ffffffffe69a594c67460f781debbca0cfc731c29d │
│                                   │ ddba613d65cf630acb00db6c93c9c0000000006b483045022100ef75d49925b7465bec5bc367f87fc7726d33aa17f472cd1ab6c13181d686139402 │
│                                   │ 20447c529336a478f4b9d89cc1453ca1cc22f34c13c3b69f7440fcc7fe889493880121022d7424c741213a2b9b49aebdaa10e84419e642a8db0a09 │
│                                   │ e359a3d4c850834846ffffffff990db7e9fa1f052aba359969b64b892cb76ff881ccd38cb75c09129e9065dbb3000000006a473044022000e7b9f1 │
│                                   │ 3c99aa71ce1b8559c2a63cec9b808767a744196e9ed0bde0b5e481a40220053e683e1efc9191207f8feb5e42646301cd2b1f2a8f9e7616e29914eb │
│                                   │ 9937c10121022d7424c741213a2b9b49aebdaa10e84419e642a8db0a09e359a3d4c850834846ffffffff41b284103365315e6fb79a7aa111295a03 │
│                                   │ 9ccc96aa90d888915f9f0eabec549b000000006b483045022100b2b3cdc669c916e88615b5d2dc3c99186fc1369879bf08b681097986b6842b6302 │
│                                   │ 2014a5c0b86d9732d4f202eb5a6d8590a37b6de013e17746c5a7be76b7c932f7390121022d7424c741213a2b9b49aebdaa10e84419e642a8db0a09 │
│                                   │ e359a3d4c850834846ffffffffdd878ff57eb64187cb74390a4b959c4012d11b766c698f706634e3360e48a6f0000000006b483045022100a16683 │
│                                   │ 4300118a432b2b4374e8d076be488b0f84109381f0862c050e73873885022050b3ca3475f1e266ab0b84f30f9030b2d4186a9939c305071691502c │
│                                   │ de007d8c0121022d7424c741213a2b9b49aebdaa10e84419e642a8db0a09e359a3d4c850834846ffffffff509a8dc208434566c92ccfcc493df990 │
│                                   │ da5edd7454c9527dd7df6d7c8aff49a8020000006b483045022100a02adf39ac6ad8e16603e0fc56948f7973223287f0fc0c9665ccd556b135193e │
│                                   │ 022065ef4363429e83ae3703f6536a9f5294b178e7f1641bd24af7bbf3d72c0ada700121022d7424c741213a2b9b49aebdaa10e84419e642a8db0a │
│                                   │ 09e359a3d4c850834846ffffffffc3fbc1f34fd64e0dcb1db65a18789f2fbb170cc41a874f8788d904a24b3c2c5d020000006a47304402201e8190 │
│                                   │ 95555707955dc508afc6db4acc7662c62460efa79a982f921bfd4afcb90220694cb9276b5228d544571cfdca849f6c18a6abf7169b983f9d47e88d │
│                                   │ a43cd4b90121022d7424c741213a2b9b49aebdaa10e84419e642a8db0a09e359a3d4c850834846ffffffff0300e1f5050000000017a914cea7345f │
│                                   │ e6ada43ef95fd6cdfd8c339ef7d1c864870000000000000000166a14a5cfc9787066562ba03c7538d024a88fd1a0fe12d08c15d460ba11001976a9 │
│                                   │ 14046922483fab8ca76b23e55e9d338605e2dbab6088ac07bebf64000000000000000000000000000000                                   │
│                                   │ tx_hash: 3284af63a9fa0c4080fd5367c3c7c1ab1d00bb12ae47eb8fb0f5e2bd2da4736a                                              │
│                                   │                                                                                                                        │
│ MakerPaymentWaitConfirmStarted    │                                                                                                                        │
│ 23-07-25 12:20:24                 │                                                                                                                        │
│                                   │                                                                                                                        │
│ MakerPaymentValidatedAndConfirmed │                                                                                                                        │
│ 23-07-25 12:20:40                 │                                                                                                                        │
│                                   │                                                                                                                        │
│ TakerPaymentSent                  │ tx_hex: 0400008085202f8901fd70e531b7c66890fe6ccb52b5b1f0a80f6c23fb0eb0e274962076e993371fc7010000006a4730440220259ab8ec │
│ 23-07-25 12:20:40                 │ 216b802f32092ef307017f183f4bd8a52bec420363abf7f070d444a8022061fce8a8b562e07b8ab41afd8953521ad7d22ffb0aa5c710f044554d89 │
│                                   │ 833bb6012102264fcd9401d797c50fe2f1c7d5fe09bbc10f3838c1d8d6f793061fa5f38b2b4dffffffff0300e1f5050000000017a914b491ff619f │
│                                   │ 632ac1b7ef4e11f64404cff0e98adf870000000000000000166a14a5cfc9787066562ba03c7538d024a88fd1a0fe12f8c8b902635600001976a914 │
│                                   │ 9934ebeaa56cb597c936a9ed8202d8d97a0a700388ac18bebf64000000000000000000000000000000                                     │
│                                   │ tx_hash: 75cbad92b60fdb6be1fc7e73b6bac9b4b531c4f14d03b5201f8ff26f20ca1e5d                                              │
│                                   │                                                                                                                        │
│ TakerPaymentSpent                 │ tx_hex: 0400008085202f89015d1eca206ff28f1f20b5034df1c431b5b4c9bab6737efce16bdb0fb692adcb7500000000d8483045022100a0ec1d │
│ 23-07-25 12:21:21                 │ 13d15a4f02a18a9adaa3442d8a9b956034c3e45b68bcbada8f877aef3b02206d59dcea375e86d5a014d51728c74a172c22a5b3cdc5dbe8daa70bb4 │
│                                   │ b887a5a30120bed41dce1b0681670b3cad9d31c862bb166fcab656e23d4c00eef7dcac38cad4004c6b63046fdcbf64b1752102264fcd9401d797c5 │
│                                   │ 0fe2f1c7d5fe09bbc10f3838c1d8d6f793061fa5f38b2b4dac6782012088a914a5cfc9787066562ba03c7538d024a88fd1a0fe128821022d7424c7 │
│                                   │ 41213a2b9b49aebdaa10e84419e642a8db0a09e359a3d4c850834846ac68ffffffff0118ddf505000000001976a914046922483fab8ca76b23e55e │
│                                   │ 9d338605e2dbab6088ac6fdcbf64000000000000000000000000000000                                                             │
│                                   │ tx_hash: 13de819b027b4ae98e730679b2b716f98bd1154f729303efd89615f152865586                                              │
│                                   │ secret: bed41dce1b0681670b3cad9d31c862bb166fcab656e23d4c00eef7dcac38cad4                                               │
│                                   │                                                                                                                        │
│ MakerPaymentSpent                 │ tx_hex: 0400008085202f89016a73a42dbde2f5b08feb47ae12bb001dabc1c7c36753fd80400cfaa963af843200000000d74730440220641be55e │
│ 23-07-25 12:21:21                 │ f769d759be59afe213d57eeeedf7d0f57bcf90835c8c3b7642d0e78902202a8f07ce745553107bea98a58cd50edb46782267fbeb4960c28073ad04 │
│                                   │ 12cc380120bed41dce1b0681670b3cad9d31c862bb166fcab656e23d4c00eef7dcac38cad4004c6b6304e6fabf64b17521022d7424c741213a2b9b │
│                                   │ 49aebdaa10e84419e642a8db0a09e359a3d4c850834846ac6782012088a914a5cfc9787066562ba03c7538d024a88fd1a0fe12882102264fcd9401 │
│                                   │ d797c50fe2f1c7d5fe09bbc10f3838c1d8d6f793061fa5f38b2b4dac68ffffffff0118ddf505000000001976a9149934ebeaa56cb597c936a9ed82 │
│                                   │ 02d8d97a0a700388ace6fabf64000000000000000000000000000000                                                               │
│                                   │ tx_hash: 4f2cc7a83d7012c5d03fa64df188500db4bee51bbb9a6a0a1f06a50ca3409fdc                                              │
│                                   │                                                                                                                        │
";
