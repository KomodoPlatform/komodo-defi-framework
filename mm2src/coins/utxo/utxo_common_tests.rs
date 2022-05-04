use super::*;
use crate::utxo::rpc_clients::{ElectrumClient, UtxoRpcClientOps};
use common::jsonrpc_client::JsonRpcErrorType;

pub async fn test_electrum_display_balances(rpc_client: &ElectrumClient) {
    let addresses = vec![
        "RG278CfeNPFtNztFZQir8cgdWexVhViYVy".into(),
        "RYPz6Lr4muj4gcFzpMdv3ks1NCGn3mkDPN".into(),
        "RJeDDtDRtKUoL8BCKdH7TNCHqUKr7kQRsi".into(),
        "RQHn9VPHBqNjYwyKfJbZCiaxVrWPKGQjeF".into(),
    ];
    let actual = rpc_client.display_balances(addresses, 8).compat().await.unwrap();

    let expected: Vec<(Address, BigDecimal)> = vec![
        ("RG278CfeNPFtNztFZQir8cgdWexVhViYVy".into(), BigDecimal::from(5.77699)),
        ("RYPz6Lr4muj4gcFzpMdv3ks1NCGn3mkDPN".into(), BigDecimal::from(0)),
        ("RJeDDtDRtKUoL8BCKdH7TNCHqUKr7kQRsi".into(), BigDecimal::from(0.77699)),
        ("RQHn9VPHBqNjYwyKfJbZCiaxVrWPKGQjeF".into(), BigDecimal::from(16.55398)),
    ];
    assert_eq!(actual, expected);

    let invalid_hashes = vec![
        "0128a4ea8c5775039d39a192f8490b35b416f2f194cb6b6ee91a41d01233c3b5".to_owned(),
        "!INVALID!".to_owned(),
        "457206aa039ed77b223e4623c19152f9aa63aa7845fe93633920607500766931".to_owned(),
    ];

    let rpc_err = rpc_client
        .scripthash_get_balances(invalid_hashes)
        .compat()
        .await
        .unwrap_err();
    match rpc_err.error {
        JsonRpcErrorType::Response(_, json_err) => {
            let expected = json!({"code": 1, "message": "!INVALID! is not a valid script hash"});
            assert_eq!(json_err, expected);
        },
        ekind => panic!("Unexpected `JsonRpcErrorType`: {:?}", ekind),
    }
}
