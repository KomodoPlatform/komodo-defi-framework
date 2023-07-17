use std::io::Write;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};

use crate::activation_scheme_db::{get_activation_scheme, get_activation_scheme_path, init_activation_scheme};
use crate::adex_config::AdexConfigImpl;
use crate::adex_proc::ResponseHandlerImpl;
use crate::cli::Cli;

const FAKE_SERVER_COOLDOWN_TIMEOUT_MS: u64 = 10;
const FAKE_SERVER_WARMUP_TIMEOUT_MS: u64 = 100;

#[tokio::test]
async fn test_get_version() {
    tokio::spawn(fake_mm2_server(7784, include_bytes!("http_mock_data/version.http")));
    tokio::time::sleep(Duration::from_millis(FAKE_SERVER_WARMUP_TIMEOUT_MS)).await;
    let mut buffer: Vec<u8> = vec![];
    let response_handler = ResponseHandlerImpl {
        writer: (&mut buffer as &mut dyn Write).into(),
    };
    let config = AdexConfigImpl::new("dummy", "http://127.0.0.1:7784");
    let args = vec!["adex-cli", "version"];
    Cli::execute(args.iter().map(|arg| arg.to_string()), &config, &response_handler)
        .await
        .unwrap();

    let result = String::from_utf8(buffer).unwrap();
    assert_eq!(
        "Version: 1.0.1-beta_824ca36f3\nDatetime: 2023-04-06T22:35:43+05:00\n",
        result
    );
}

#[tokio::test]
async fn test_get_orderbook() {
    tokio::spawn(fake_mm2_server(7785, include_bytes!("http_mock_data/orderbook.http")));
    tokio::time::sleep(Duration::from_millis(FAKE_SERVER_WARMUP_TIMEOUT_MS)).await;
    let mut buffer: Vec<u8> = vec![];
    let response_handler = ResponseHandlerImpl {
        writer: (&mut buffer as &mut dyn Write).into(),
    };
    let config = AdexConfigImpl::new("dummy", "http://127.0.0.1:7785");
    let args = vec!["adex-cli", "orderbook", "RICK", "MORTY"];
    Cli::execute(args.iter().map(|arg| arg.to_string()), &config, &response_handler)
        .await
        .unwrap();

    let result = String::from_utf8(buffer).unwrap();
    assert_eq!(RICK_AND_MORTY_ORDERBOOK, result);
}

#[tokio::test]
async fn test_get_orderbook_with_uuids() {
    tokio::spawn(fake_mm2_server(7786, include_bytes!("http_mock_data/orderbook.http")));
    tokio::time::sleep(Duration::from_millis(FAKE_SERVER_WARMUP_TIMEOUT_MS)).await;
    let mut buffer: Vec<u8> = vec![];
    let response_handler = ResponseHandlerImpl {
        writer: (&mut buffer as &mut dyn Write).into(),
    };
    let config = AdexConfigImpl::new("dummy", "http://127.0.0.1:7786");
    let args = vec!["adex-cli", "orderbook", "RICK", "MORTY", "--uuids"];
    Cli::execute(args.iter().map(|arg| arg.to_string()), &config, &response_handler)
        .await
        .unwrap();

    let result = String::from_utf8(buffer).unwrap();
    assert_eq!(RICK_AND_MORTY_ORDERBOOK_WITH_UUIDS, result);
}

#[tokio::test]
async fn test_get_orderbook_with_publics() {
    tokio::spawn(fake_mm2_server(7787, include_bytes!("http_mock_data/orderbook.http")));
    tokio::time::sleep(Duration::from_millis(FAKE_SERVER_WARMUP_TIMEOUT_MS)).await;
    let mut buffer: Vec<u8> = vec![];
    let response_handler = ResponseHandlerImpl {
        writer: (&mut buffer as &mut dyn Write).into(),
    };
    let config = AdexConfigImpl::new("dummy", "http://127.0.0.1:7787");
    let args = vec!["adex-cli", "orderbook", "RICK", "MORTY", "--publics"];
    Cli::execute(args.iter().map(|arg| arg.to_string()), &config, &response_handler)
        .await
        .unwrap();

    let result = String::from_utf8(buffer).unwrap();
    assert_eq!(RICK_AND_MORTY_ORDERBOOK_WITH_PUBLICS, result);
}

#[tokio::test]
async fn test_get_enabled() {
    tokio::spawn(fake_mm2_server(7788, include_bytes!("http_mock_data/get_enabled.http")));
    tokio::time::sleep(Duration::from_millis(FAKE_SERVER_WARMUP_TIMEOUT_MS)).await;
    let mut buffer: Vec<u8> = vec![];
    let response_handler = ResponseHandlerImpl {
        writer: (&mut buffer as &mut dyn Write).into(),
    };
    let config = AdexConfigImpl::new("dummy", "http://127.0.0.1:7788");
    let args = vec!["adex-cli", "get-enabled"];
    Cli::execute(args.iter().map(|arg| arg.to_string()), &config, &response_handler)
        .await
        .unwrap();

    let result = String::from_utf8(buffer).unwrap();
    assert_eq!(ENABLED_COINS, result);
}

#[tokio::test]
async fn test_get_balance() {
    tokio::spawn(fake_mm2_server(7789, include_bytes!("http_mock_data/balance.http")));
    tokio::time::sleep(Duration::from_millis(FAKE_SERVER_WARMUP_TIMEOUT_MS)).await;
    let mut buffer: Vec<u8> = vec![];
    let response_handler = ResponseHandlerImpl {
        writer: (&mut buffer as &mut dyn Write).into(),
    };
    let config = AdexConfigImpl::new("dummy", "http://127.0.0.1:7789");
    let args = vec!["adex-cli", "balance", "RICK"];
    Cli::execute(args.iter().map(|arg| arg.to_string()), &config, &response_handler)
        .await
        .unwrap();

    let result = String::from_utf8(buffer).unwrap();
    assert_eq!(RICK_BALANCE, result);
}

#[tokio::test]
async fn test_enable() {
    tokio::spawn(fake_mm2_server(7790, include_bytes!("http_mock_data/enable.http")));
    test_activation_scheme().await;
    tokio::time::sleep(Duration::from_millis(FAKE_SERVER_WARMUP_TIMEOUT_MS)).await;
    let mut buffer: Vec<u8> = vec![];
    let response_handler = ResponseHandlerImpl {
        writer: (&mut buffer as &mut dyn Write).into(),
    };
    let config = AdexConfigImpl::new("dummy", "http://127.0.0.1:7790");
    let args = vec!["adex-cli", "enable", "ETH"];
    Cli::execute(args.iter().map(|arg| arg.to_string()), &config, &response_handler)
        .await
        .unwrap();

    let result = String::from_utf8(buffer).unwrap();
    assert_eq!(ENABLE_OUTPUT, result);
}

async fn test_activation_scheme() {
    let _ = std::fs::remove_file(get_activation_scheme_path().unwrap());
    init_activation_scheme().await.unwrap();
    let scheme = get_activation_scheme().unwrap();
    let kmd_scheme = scheme.get_activation_method("KMD");
    assert!(kmd_scheme.is_some());
    let kmd_scheme = kmd_scheme.unwrap();
    assert_eq!(kmd_scheme.get("method").unwrap().as_str().unwrap(), "electrum");
    assert_ne!(kmd_scheme.get("servers").unwrap().as_array().unwrap().iter().count(), 0);
}

#[tokio::test]
async fn test_buy_morty_for_rick() {
    tokio::spawn(fake_mm2_server(7791, include_bytes!("http_mock_data/buy.http")));
    tokio::time::sleep(Duration::from_millis(FAKE_SERVER_WARMUP_TIMEOUT_MS)).await;
    let mut buffer: Vec<u8> = vec![];
    let response_handler = ResponseHandlerImpl {
        writer: (&mut buffer as &mut dyn Write).into(),
    };
    let config = AdexConfigImpl::new("dummy", "http://127.0.0.1:7791");
    let args = vec!["adex-cli", "buy", "MORTY", "RICK", "0.01", "0.5"];
    Cli::execute(args.iter().map(|arg| arg.to_string()), &config, &response_handler)
        .await
        .unwrap();

    let result = String::from_utf8(buffer).unwrap();
    assert_eq!("Buy order uuid: 4685e133-dfb3-4b31-8d4c-0ffa79933c8e\n", result);
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

const RICK_AND_MORTY_ORDERBOOK: &str = r"     Volume: RICK Price: MORTY  
             0.23 1.00000000    
        340654.03 1.00000000    
             2.00 0.99999999    
             2.00 0.99999999    
             2.00 0.99999999    
- --------------- ------------- 
             0.96 1.02438024    
             1.99 1.00000001    
             1.99 1.00000001    
             1.99 1.00000001    
         32229.14 1.00000000    
             0.22 1.00000000    
";

const RICK_AND_MORTY_ORDERBOOK_WITH_UUIDS: &str = r"     Volume: RICK Price: MORTY  Uuid                                 
             0.23 1.00000000    c7585a1b-6060-4319-9da6-c67321628a06 
        340654.03 1.00000000    d69fe2a9-51ca-4d69-96ad-b141a01d8bb4 
             2.00 0.99999999    a2337218-7f6f-46a1-892e-6febfb7f5403 
             2.00 0.99999999    c172c295-7fe3-4131-9c81-c3a7182f0617 
             2.00 0.99999999    fbbc44d2-fb50-4b4b-8ac3-d9857cae16b6 
- --------------- ------------- ------------------------------------ 
             0.96 1.02438024    c480675b-3352-4159-9b3c-55cb2b1329de 
             1.99 1.00000001    fdb0de9c-e283-48c3-9de6-8117fecf0aff 
             1.99 1.00000001    6a3bb75d-8e91-4192-bf50-d8190a69600d 
             1.99 1.00000001    b24b40de-e93d-4218-8d93-1940ceadce7f 
         32229.14 1.00000000    652a7e97-f42c-4f87-bc26-26bd1a0fea24 
             0.22 1.00000000    1082c93c-8c23-4944-b8f1-a92ec703b03a 
";

const RICK_AND_MORTY_ORDERBOOK_WITH_PUBLICS: &str = r"     Volume: RICK Price: MORTY  Public                                                             
             0.23 1.00000000    022d7424c741213a2b9b49aebdaa10e84419e642a8db0a09e359a3d4c850834846 
        340654.03 1.00000000    0315d9c51c657ab1be4ae9d3ab6e76a619d3bccfe830d5363fa168424c0d044732 
             2.00 0.99999999    037310a8fb9fd8f198a1a21db830252ad681fccda580ed4101f3f6bfb98b34fab5 
             2.00 0.99999999    037310a8fb9fd8f198a1a21db830252ad681fccda580ed4101f3f6bfb98b34fab5 
             2.00 0.99999999    037310a8fb9fd8f198a1a21db830252ad681fccda580ed4101f3f6bfb98b34fab5 
- --------------- ------------- ------------------------------------------------------------------ 
             0.96 1.02438024    02d6c3e22a419a4034272acb215f1d39cd6a0413cfd83ac0c68f482db80accd89a 
             1.99 1.00000001    037310a8fb9fd8f198a1a21db830252ad681fccda580ed4101f3f6bfb98b34fab5 
             1.99 1.00000001    037310a8fb9fd8f198a1a21db830252ad681fccda580ed4101f3f6bfb98b34fab5 
             1.99 1.00000001    037310a8fb9fd8f198a1a21db830252ad681fccda580ed4101f3f6bfb98b34fab5 
         32229.14 1.00000000    0315d9c51c657ab1be4ae9d3ab6e76a619d3bccfe830d5363fa168424c0d044732 
             0.22 1.00000000    022d7424c741213a2b9b49aebdaa10e84419e642a8db0a09e359a3d4c850834846 
";

const ENABLED_COINS: &str = r"Ticker   Address
MORTY    RPFGrvJWjSYN4qYvcXsECW1HoHbvQjowZM
RICK     RPFGrvJWjSYN4qYvcXsECW1HoHbvQjowZM
KMD      RPFGrvJWjSYN4qYvcXsECW1HoHbvQjowZM
ETH      0x224050fb7EB13Fa0D342F5b245f1237bAB531650
";

const RICK_BALANCE: &str = r"coin: RICK
balance: 0.5767226
unspendable: 0
address: RPFGrvJWjSYN4qYvcXsECW1HoHbvQjowZM
";

const ENABLE_OUTPUT: &str = r"coin: ETH
address: 0x224050fb7EB13Fa0D342F5b245f1237bAB531650
balance: 0.02
unspendable_balance: 0
required_confirmations: 3
requires_notarization: No
";
