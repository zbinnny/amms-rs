use async_trait::async_trait;
use ethers::types::{Log, H160, H256, U256};
use serde::{Deserialize, Serialize};

use crate::currency::Currency;
use crate::errors::{ArithmeticError, EventLogError, SwapSimulationError};

use self::uniswap_v2::UniswapV2Pool;

pub mod factory;
pub mod uniswap_v2;

#[async_trait]
pub trait AutomatedMarketMaker {
    /// Returns the address of the AMM.
    fn address(&self) -> H160;

    /// 返回池子相关的所有货币地址
    fn tokens(&self) -> Vec<H160>;

    /// 返回这个池子相关的所有货币信息
    fn currencies(&self) -> Vec<Currency>;

    fn set_currency(&mut self, currency: Currency);

    /// 最后同步的日志
    fn last_synced_log(&self) -> (u64, u64);

    /// Returns the vector of event signatures subscribed to when syncing the AMM.
    fn sync_on_event_signatures(&self) -> Vec<H256>;

    /// Updates the AMM data from a log.
    fn sync_from_log(&mut self, log: Log) -> Result<(), EventLogError>;

    /// Calculates a f64 representation of base token price in the AMM.
    fn calculate_price(&self, base_token: H160) -> Result<f64, ArithmeticError>;

    /// Returns the token out of the AMM for a given `token_in`.
    fn get_token_out(&self, token_in: H160) -> H160;

    /// Locally simulates a swap in the AMM.
    ///
    /// Returns the amount received for `amount_in` of `token_in`.
    fn simulate_swap(&self, token_in: H160, amount_in: U256) -> Result<U256, SwapSimulationError>;

    /// Locally simulates a swap in the AMM.
    /// Mutates the AMM state to the state of the AMM after swapping.
    /// Returns the amount received for `amount_in` of `token_in`.
    fn simulate_swap_mut(
        &mut self,
        token_in: H160,
        amount_in: U256,
    ) -> Result<U256, SwapSimulationError>;
}

macro_rules! amm {
    ($($pool_type:ident),+ $(,)?) => {
        #[derive(Debug, Clone, Serialize, Deserialize)]
        pub enum AMM {
            $($pool_type($pool_type),)+
        }

        #[async_trait]
        impl AutomatedMarketMaker for AMM {
            fn address(&self) -> H160 {
                match self {
                    $(AMM::$pool_type(pool) => pool.address(),)+
                }
            }

            fn tokens(&self) -> Vec<H160> {
                match self {
                    $(AMM::$pool_type(pool) => pool.tokens(),)+
                }
            }

            fn currencies(&self) -> Vec<Currency> {
                match self {
                    $(AMM::$pool_type(pool) => pool.currencies(),)+
                }
            }

            fn set_currency(&mut self, currency: Currency) {
                match self {
                    $(AMM::$pool_type(pool) => pool.set_currency(currency),)+
                }
            }

            fn last_synced_log(&self) -> (u64, u64) {
                match self {
                    $(AMM::$pool_type(pool) => pool.last_synced_log(),)+
                }
            }

            fn sync_on_event_signatures(&self) -> Vec<H256> {
                match self {
                    $(AMM::$pool_type(pool) => pool.sync_on_event_signatures(),)+
                }
            }

            fn sync_from_log(&mut self, log: Log) -> Result<(), EventLogError> {
                match self {
                    $(AMM::$pool_type(pool) => pool.sync_from_log(log),)+
                }
            }

            fn calculate_price(&self, base_token: H160) -> Result<f64, ArithmeticError> {
                match self {
                    $(AMM::$pool_type(pool) => pool.calculate_price(base_token),)+
                }
            }

            fn get_token_out(&self, token_in: H160) -> H160 {
                match self {
                    $(AMM::$pool_type(pool) => pool.get_token_out(token_in),)+
                }
            }

            fn simulate_swap(&self, token_in: H160, amount_in: U256) -> Result<U256, SwapSimulationError> {
                match self {
                    $(AMM::$pool_type(pool) => pool.simulate_swap(token_in, amount_in),)+
                }
            }

            fn simulate_swap_mut(&mut self, token_in: H160, amount_in: U256) -> Result<U256, SwapSimulationError> {
                match self {
                    $(AMM::$pool_type(pool) => pool.simulate_swap_mut(token_in, amount_in),)+
                }
            }
        }
    };
}

amm!(UniswapV2Pool);
