use clap::Subcommand;

#[path = "commands_order/cmd_best_orders.rs"]
mod cmd_best_orders;
#[path = "commands_order/cmd_order_status.rs"]
mod cmd_order_status;
#[path = "commands_order/cmd_orderbook.rs"] mod cmd_orderbook;
#[path = "commands_order/cmd_orderbook_depth.rs"]
mod cmd_orderbook_depth;
#[path = "commands_order/cmd_orders_history.rs"]
mod cmd_orders_history;

pub(crate) use cmd_best_orders::BestOrderArgs;
pub(crate) use cmd_order_status::OrderStatusArgs;
pub(crate) use cmd_orderbook::OrderbookArgs;
pub(crate) use cmd_orderbook_depth::OrderbookDepthArgs;
pub(crate) use cmd_orders_history::OrdersHistoryArgs;

#[allow(clippy::large_enum_variant)]
#[derive(Subcommand)]
pub(crate) enum OrderCommands {
    #[command(visible_aliases = ["book"], about = "Get orderbook")]
    Orderbook(OrderbookArgs),
    #[command(about = "Get orderbook depth")]
    OrderbookDepth(OrderbookDepthArgs),
    #[command(
        visible_alias = "status",
        about = "Return the data of the order with the given uuid created by the current node"
    )]
    OrderStatus(OrderStatusArgs),
    #[command(
        visible_alias = "best",
        about = "Return the best priced trades available on the orderbook"
    )]
    BestOrders(BestOrderArgs),
    #[command(about = "Get my orders", visible_aliases = ["my", "mine"])]
    MyOrders,
    #[command(
        visible_aliases = ["history", "filter"],
        about = "Return all orders whether active or inactive that match the given filters"
    )]
    OrdersHistory(OrdersHistoryArgs),
}
