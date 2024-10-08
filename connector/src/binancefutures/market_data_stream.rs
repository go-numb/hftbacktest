use std::collections::HashMap;

use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use hftbacktest::{live::Instrument, prelude::*};
use tokio::{
    select,
    sync::{
        broadcast::{error::RecvError, Receiver},
        mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
    },
};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
};
use tracing::error;

use crate::{
    binancefutures::{
        msg::{rest, stream, stream::Stream},
        rest::BinanceFuturesClient,
        BinanceFuturesError,
    },
    connector::PublishMessage,
    utils::{parse_depth, parse_px_qty_tup},
};

pub struct MarketDataStream {
    symbols: HashMap<String, Instrument>,
    client: BinanceFuturesClient,
    ev_tx: UnboundedSender<PublishMessage>,
    symbol_rx: Receiver<String>,
    pending_depth_messages: HashMap<String, Vec<stream::Depth>>,
    prev_u: HashMap<String, i64>,
    rest_tx: UnboundedSender<(String, rest::Depth)>,
    rest_rx: UnboundedReceiver<(String, rest::Depth)>,
}

impl MarketDataStream {
    pub fn new(
        client: BinanceFuturesClient,
        ev_tx: UnboundedSender<PublishMessage>,
        symbol_rx: Receiver<String>,
    ) -> Self {
        let (rest_tx, rest_rx) = unbounded_channel::<(String, rest::Depth)>();
        Self {
            symbols: Default::default(),
            client,
            ev_tx,
            symbol_rx,
            pending_depth_messages: Default::default(),
            prev_u: Default::default(),
            rest_tx,
            rest_rx,
        }
    }

    fn process_message(&mut self, stream: Stream) {
        match stream {
            Stream::DepthUpdate(data) => {
                let mut prev_u_val = self.prev_u.get_mut(&data.symbol.to_lowercase());
                if prev_u_val.is_none()
                /* fixme: || data.prev_update_id != **prev_u_val.as_ref().unwrap()*/
                {
                    // if !pending_depth_messages.contains_key(&data.symbol.to_lowercase()) {
                    let client_ = self.client.clone();
                    let symbol = data.symbol.to_lowercase();
                    let rest_tx = self.rest_tx.clone();
                    tokio::spawn(async move {
                        let resp = client_.get_depth(&symbol).await;
                        match resp {
                            Ok(depth) => {
                                rest_tx.send((symbol, depth)).unwrap();
                            }
                            Err(error) => {
                                error!(
                                    ?error,
                                    %symbol,
                                    "Couldn't get the market depth via REST."
                                );
                            }
                        }
                    });
                    // }
                    // pending_depth_messages
                    //     .entry(data.symbol.clone())
                    //     .or_insert(Vec::new())
                    //     .push(data);
                    // continue;
                }
                // *prev_u_val.unwrap() = data.last_update_id;
                // fixme: currently supports natural refresh only.
                *self
                    .prev_u
                    .entry(data.symbol.to_lowercase())
                    .or_insert(data.last_update_id) = data.last_update_id;

                match parse_depth(data.bids, data.asks) {
                    Ok((bids, asks)) => {
                        for (px, qty) in bids {
                            self.ev_tx
                                .send(PublishMessage::LiveEvent(LiveEvent::Feed {
                                    symbol: data.symbol.to_lowercase(),
                                    event: Event {
                                        ev: LOCAL_BID_DEPTH_EVENT,
                                        exch_ts: data.transaction_time * 1_000_000,
                                        local_ts: Utc::now().timestamp_nanos_opt().unwrap(),
                                        order_id: 0,
                                        px,
                                        qty,
                                        ival: 0,
                                        fval: 0.0,
                                    },
                                }))
                                .unwrap();
                        }

                        for (px, qty) in asks {
                            self.ev_tx
                                .send(PublishMessage::LiveEvent(LiveEvent::Feed {
                                    symbol: data.symbol.to_lowercase(),
                                    event: Event {
                                        ev: LOCAL_ASK_DEPTH_EVENT,
                                        exch_ts: data.transaction_time * 1_000_000,
                                        local_ts: Utc::now().timestamp_nanos_opt().unwrap(),
                                        order_id: 0,
                                        px,
                                        qty,
                                        ival: 0,
                                        fval: 0.0,
                                    },
                                }))
                                .unwrap();
                        }
                    }
                    Err(error) => {
                        error!(?error, "Couldn't parse DepthUpdate stream.");
                    }
                }
            }
            Stream::Trade(data) => match parse_px_qty_tup(data.price, data.qty) {
                Ok((px, qty)) => {
                    self.ev_tx
                        .send(PublishMessage::LiveEvent(LiveEvent::Feed {
                            symbol: data.symbol.to_lowercase(),
                            event: Event {
                                ev: {
                                    if data.is_the_buyer_the_market_maker {
                                        LOCAL_SELL_TRADE_EVENT
                                    } else {
                                        LOCAL_BUY_TRADE_EVENT
                                    }
                                },
                                exch_ts: data.transaction_time * 1_000_000,
                                local_ts: Utc::now().timestamp_nanos_opt().unwrap(),
                                order_id: 0,
                                px,
                                qty,
                                ival: 0,
                                fval: 0.0,
                            },
                        }))
                        .unwrap();
                }
                Err(e) => {
                    error!(error = ?e, "Couldn't parse trade stream.");
                }
            },
            _ => unreachable!(),
        }
    }

    fn process_snapshot(&self, symbol: String, data: rest::Depth) {
        match parse_depth(data.bids, data.asks) {
            Ok((bids, asks)) => {
                for (px, qty) in bids {
                    self.ev_tx
                        .send(PublishMessage::LiveEvent(LiveEvent::Feed {
                            symbol: symbol.clone(),
                            event: Event {
                                ev: LOCAL_BID_DEPTH_EVENT,
                                exch_ts: data.transaction_time * 1_000_000,
                                local_ts: Utc::now().timestamp_nanos_opt().unwrap(),
                                order_id: 0,
                                px,
                                qty,
                                ival: 0,
                                fval: 0.0,
                            },
                        }))
                        .unwrap();
                }

                for (px, qty) in asks {
                    self.ev_tx
                        .send(PublishMessage::LiveEvent(LiveEvent::Feed {
                            symbol: symbol.clone(),
                            event: Event {
                                ev: LOCAL_ASK_DEPTH_EVENT,
                                exch_ts: data.transaction_time * 1_000_000,
                                local_ts: Utc::now().timestamp_nanos_opt().unwrap(),
                                order_id: 0,
                                px,
                                qty,
                                ival: 0,
                                fval: 0.0,
                            },
                        }))
                        .unwrap();
                }
            }
            Err(error) => {
                error!(?error, "Couldn't parse Depth response.");
            }
        }
        // fixme: waits for pending messages without blocking.
        // prev_u.remove(&symbol);
        // let mut new_prev_u: Option<i64> = None;
        // while new_prev_u.is_none() {
        //     if let Some(msg) = pending_depth_messages.get_mut(&symbol) {
        //         for pending_depth in msg.into_iter() {
        //             // https://binance-docs.github.io/apidocs/futures/en/#how-to-manage-a-local-order-book-correctly
        //             // The first processed event should have U <= lastUpdateId AND u >= lastUpdateId
        //             if (
        //                 pending_depth.last_update_id < resp.last_update_id
        //                 || pending_depth.first_update_id > resp.last_update_id
        //             ) && new_prev_u.is_none() {
        //                 continue;
        //             }
        //             if new_prev_u.is_some() && pending_depth.prev_update_id != *new_prev_u.as_ref().unwrap() {
        //                 warn!(%symbol, ?pending_depth, "UpdateId does not match.");
        //             }
        //
        //             // Processes a pending depth message
        //             new_prev_u = Some(pending_depth.last_update_id);
        //             *prev_u.entry(symbol.clone())
        //                 .or_insert(pending_depth.last_update_id) = pending_depth.last_update_id;
        //         }
        //     }
        //     if new_prev_u.is_none() {
        //         // Waits for depth messages.
        //         todo!()
        //     }
        // }
    }

    pub async fn connect(&mut self, url: &str) -> Result<(), BinanceFuturesError> {
        let request = url.into_client_request()?;
        let (ws_stream, _) = connect_async(request).await?;
        let (mut write, mut read) = ws_stream.split();

        loop {
            select! {
                Some((symbol, data)) = self.rest_rx.recv() => {
                    self.process_snapshot(symbol, data);
                }
                msg = self.symbol_rx.recv() => match msg {
                    Ok(symbol) => {
                        write.send(Message::Text(format!(r#"{{
                            "method": "SUBSCRIBE",
                            "params": [
                                "{symbol}@trade",
                                "{symbol}@depth@0ms"
                            ],
                            "id": 1
                        }}"#))).await?;
                    }
                    Err(RecvError::Closed) => {
                        return Ok(());
                    }
                    Err(RecvError::Lagged(num)) => {
                        error!("{num} subscription requests were missed.");
                    }
                },
                message = read.next() => match message {
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<Stream>(&text) {
                            Ok(stream) => {
                                self.process_message(stream);
                            }
                            Err(error) => {
                                error!(?error, %text, "Couldn't parse Stream.");
                            }
                        }
                    }
                    Some(Ok(Message::Ping(_))) => {
                        write.send(Message::Pong(Vec::new())).await?;
                    }
                    Some(Ok(Message::Close(close_frame))) => {
                        return Err(BinanceFuturesError::ConnectionAbort(
                            close_frame.map(|f| f.to_string()).unwrap_or(String::new())
                        ));
                    }
                    Some(Ok(Message::Binary(_)))
                    | Some(Ok(Message::Frame(_)))
                    | Some(Ok(Message::Pong(_))) => {}
                    Some(Err(error)) => {
                        return Err(BinanceFuturesError::from(error));
                    }
                    None => {
                        return Err(BinanceFuturesError::ConnectionInterrupted);
                    }
                }
            }
        }
    }
}
