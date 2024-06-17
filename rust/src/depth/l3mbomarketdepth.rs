use std::collections::{hash_map::Entry, BTreeMap, HashMap};

use crate::{
    backtest::BacktestError,
    depth::{L3MarketDepth, INVALID_MAX, INVALID_MIN},
    types::{Side, BUY, SELL},
};
use crate::depth::MarketDepth;

pub struct MarketOrder {
    order_id: i64,
    side: Side,
    price_tick: i32,
    qty: f32,
}

pub struct L3MBOMarketDepth {
    pub tick_size: f32,
    pub lot_size: f32,
    pub timestamp: i64,
    pub bid_depth: BTreeMap<i32, f32>,
    pub ask_depth: BTreeMap<i32, f32>,
    pub orders: HashMap<i64, MarketOrder>,
    pub best_bid_tick: i32,
    pub best_ask_tick: i32,
}

impl L3MBOMarketDepth {
    pub fn add(&mut self, order: MarketOrder) -> Result<(), BacktestError> {
        if order.side == Side::Buy {
            *self.bid_depth.entry(order.price_tick).or_insert(0.0) += order.qty;
        } else {
            *self.ask_depth.entry(order.price_tick).or_insert(0.0) += order.qty;
        }
        match self.orders.entry(order.order_id) {
            Entry::Occupied(_) => Err(BacktestError::OrderIdExist),
            Entry::Vacant(entry) => {
                entry.insert(order);
                Ok(())
            }
        }
    }
}

impl L3MarketDepth for L3MBOMarketDepth {
    type Error = BacktestError;

    fn add_buy_order(
        &mut self,
        order_id: i64,
        px: f32,
        qty: f32,
        timestamp: i64,
    ) -> Result<(i32, i32), Self::Error> {
        let price_tick = (px / self.tick_size).round() as i32;
        self.add(MarketOrder {
            order_id,
            side: Side::Buy,
            price_tick,
            qty,
        })?;
        let prev_best_tick = self.best_bid_tick;
        if price_tick > self.best_bid_tick {
            self.best_bid_tick = *self.bid_depth.keys().last().unwrap_or(&INVALID_MIN);
        }
        Ok((prev_best_tick, self.best_bid_tick))
    }

    fn add_sell_order(
        &mut self,
        order_id: i64,
        px: f32,
        qty: f32,
        timestamp: i64,
    ) -> Result<(i32, i32), Self::Error> {
        let price_tick = (px / self.tick_size).round() as i32;
        self.add(MarketOrder {
            order_id,
            side: Side::Sell,
            price_tick,
            qty,
        })?;
        let prev_best_tick = self.best_ask_tick;
        if price_tick < self.best_ask_tick {
            self.best_ask_tick = *self.ask_depth.keys().next().unwrap_or(&INVALID_MAX);
        }
        Ok((prev_best_tick, self.best_ask_tick))
    }

    fn delete_order(&mut self, order_id: i64, timestamp: i64) -> Result<(), Self::Error> {
        let order = self
            .orders
            .remove(&order_id)
            .ok_or(BacktestError::OrderNotFound)?;
        if order.side == Side::Buy {
            let depth_qty = self.bid_depth.get_mut(&order.price_tick).unwrap();
            *depth_qty -= order.qty;
            if (*depth_qty / self.lot_size as f32).round() as i32 == 0 {
                self.bid_depth.remove(&order.price_tick).unwrap();
            }
        } else {
            let depth_qty = self.ask_depth.get_mut(&order.price_tick).unwrap();
            *depth_qty -= order.qty;
            if (*depth_qty / self.lot_size as f32).round() as i32 == 0 {
                self.ask_depth.remove(&order.price_tick).unwrap();
            }
        }
        Ok(())
    }

    fn modify_order(
        &mut self,
        order_id: i64,
        px: f32,
        qty: f32,
        timestamp: i64,
    ) -> Result<(i64, i32, i32), Self::Error> {
        let order = self
            .orders
            .get_mut(&order_id)
            .ok_or(BacktestError::OrderNotFound)?;
        if order.side == Side::Buy {
            let price_tick = (px / self.tick_size).round() as i32;
            if price_tick != order.price_tick {
                let depth_qty = self.bid_depth.get_mut(&order.price_tick).unwrap();
                *depth_qty -= order.qty;
                if (*depth_qty / self.lot_size).round() as i32 == 0 {
                    self.bid_depth.remove(&order.price_tick).unwrap();
                }

                order.price_tick = price_tick;
                order.qty = qty;

                *self.bid_depth.entry(order.price_tick).or_insert(0.0) += order.qty;

                let prev_best_tick = self.best_bid_tick;
                if price_tick > self.best_bid_tick {
                    self.best_bid_tick = *self.bid_depth.keys().last().unwrap_or(&INVALID_MIN);
                }
                Ok((BUY, prev_best_tick, self.best_bid_tick))
            } else {
                let depth_qty = self.bid_depth.get_mut(&order.price_tick).unwrap();
                *depth_qty += qty - order.qty;
                order.qty = qty;
                Ok((BUY, self.best_bid_tick, self.best_bid_tick))
            }
        } else {
            let price_tick = (px / self.tick_size).round() as i32;
            if price_tick != order.price_tick {
                let depth_qty = self.ask_depth.get_mut(&order.price_tick).unwrap();
                *depth_qty -= order.qty;
                if (*depth_qty / self.lot_size).round() as i32 == 0 {
                    self.bid_depth.remove(&order.price_tick).unwrap();
                }

                order.price_tick = price_tick;
                order.qty = qty;

                *self.ask_depth.entry(order.price_tick).or_insert(0.0) += order.qty;

                let prev_best_tick = self.best_ask_tick;
                if price_tick < self.best_ask_tick {
                    self.best_ask_tick = *self.ask_depth.keys().next().unwrap_or(&INVALID_MAX);
                }
                Ok((SELL, prev_best_tick, self.best_ask_tick))
            } else {
                let depth_qty = self.ask_depth.get_mut(&order.price_tick).unwrap();
                *depth_qty += qty - order.qty;
                order.qty = qty;
                Ok((SELL, self.best_ask_tick, self.best_ask_tick))
            }
        }
    }
}

impl MarketDepth for L3MBOMarketDepth {
    fn update_bid_depth(&mut self, price: f32, qty: f32, timestamp: i64) -> (i32, i32, i32, f32, f32, i64) {
        todo!()
    }

    fn update_ask_depth(&mut self, price: f32, qty: f32, timestamp: i64) -> (i32, i32, i32, f32, f32, i64) {
        todo!()
    }

    fn clear_depth(&mut self, side: i64, clear_upto_price: f32) {
        if side == BUY {
            self.bid_depth.clear();
        } else {
            self.ask_depth.clear();
        }
    }

    #[inline(always)]
    fn best_bid(&self) -> f32 {
        self.best_bid_tick() as f32 * self.tick_size
    }

    #[inline(always)]
    fn best_ask(&self) -> f32 {
        self.best_ask_tick() as f32 * self.tick_size
    }

    #[inline(always)]
    fn best_bid_tick(&self) -> i32 {
        *self.bid_depth.keys().last().unwrap_or(&INVALID_MIN)
    }

    #[inline(always)]
    fn best_ask_tick(&self) -> i32 {
        *self.ask_depth.keys().next().unwrap_or(&INVALID_MAX)
    }

    #[inline(always)]
    fn tick_size(&self) -> f32 {
        self.tick_size
    }

    #[inline(always)]
    fn lot_size(&self) -> f32 {
        self.lot_size
    }

    #[inline(always)]
    fn bid_qty_at_tick(&self, price_tick: i32) -> f32 {
        *self.bid_depth.get(&price_tick).unwrap_or(&0.0)
    }

    #[inline(always)]
    fn ask_qty_at_tick(&self, price_tick: i32) -> f32 {
        *self.ask_depth.get(&price_tick).unwrap_or(&0.0)
    }
}