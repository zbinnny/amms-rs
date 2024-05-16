use async_trait::async_trait;
use ethers::{
    abi::{ethabi::Bytes, RawLog, Token},
    prelude::{abigen, EthEvent},
    types::{H160, H256, Log, U256},
};
use num_bigfloat::BigFloat;
use ruint::Uint;
use serde::{Deserialize, Serialize};

pub use constant::*;

use crate::{
    amm::AutomatedMarketMaker,
    errors::{ArithmeticError, EventLogError, SwapSimulationError},
};
use crate::currency::Currency;

use self::factory::PAIR_CREATED_EVENT_SIGNATURE;

pub mod batch_request;
pub mod constant;
pub mod factory;

abigen!(
    IUniswapV2Pair,
    r#"[
        function getReserves() external view returns (uint112 reserve0, uint112 reserve1, uint32 blockTimestampLast)
        function token0() external view returns (address)
        function token1() external view returns (address)
        function swap(uint256 amount0Out, uint256 amount1Out, address to, bytes calldata data);
        event Sync(uint112 reserve0, uint112 reserve1)
    ]"#;

    IErc20,
    r#"[
        function balanceOf(address account) external view returns (uint256)
        function decimals() external view returns (uint8)
    ]"#;
);

pub const U128_0X10000000000000000: u128 = 18446744073709551616;
pub const SYNC_EVENT_SIGNATURE: H256 = H256([
    28, 65, 30, 154, 150, 224, 113, 36, 28, 47, 33, 247, 114, 107, 23, 174, 137, 227, 202, 180,
    199, 139, 229, 14, 6, 43, 3, 169, 255, 251, 186, 209,
]);

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UniswapV2Pool {
    pub address: H160,
    pub token_a: Currency,
    pub token_b: Currency,
    pub reserve_0: u128,
    pub reserve_1: u128,
    pub last_synced_log: (u64, u64),
    pub fee: u32,
}

#[async_trait]
impl AutomatedMarketMaker for UniswapV2Pool {
    fn address(&self) -> H160 {
        self.address
    }

    fn tokens(&self) -> Vec<H160> {
        vec![self.token_a.address(), self.token_b.address()]
    }

    fn currencies(&self) -> Vec<Currency> {
        vec![self.token_a.clone(), self.token_b.clone()]
    }

    fn reserves(&self) -> Vec<u128> {
        vec![self.reserve_0, self.reserve_1]
    }

    fn set_currency(&mut self, currency: Currency) {
        match currency.address() {
            v if v == self.token_a.address() => {
                self.token_a = currency;
            }
            v if v == self.token_b.address() => {
                self.token_b = currency;
            }
            _ => {}
        }
    }

    fn last_synced_log(&self) -> (u64, u64) {
        self.last_synced_log
    }

    fn data_is_populated(&self) -> bool {
        self.token_a.data_is_populated()
            && self.token_b.data_is_populated()
            && self.last_synced_log != (0, 0)
            && self.reserve_0 != 0
            && self.reserve_1 != 0
    }

    fn sync_on_event_signatures(&self) -> Vec<H256> {
        vec![SYNC_EVENT_SIGNATURE]
    }

    fn sync_from_log(&mut self, log: Log) -> Result<(), EventLogError> {
        let event_signature = log.topics[0];

        if event_signature == SYNC_EVENT_SIGNATURE {
            let new_log_index = (
                log.block_number.clone().unwrap().as_u64(),
                log.log_index.clone().unwrap().as_u64(),
            );

            if new_log_index <= self.last_synced_log {
                tracing::info!(log = ?new_log_index, last_synced = ?self.last_synced_log, "Skipping sync event");
                return Err(EventLogError::LogAlreadySynced);
            }

            let sync_event = SyncFilter::decode_log(&RawLog::from(log))?;
            tracing::debug!(log_index = ?new_log_index, reserve_0 = sync_event.reserve_0, reserve_1 = sync_event.reserve_1, address = ?self.address, "UniswapV2 sync event");

            self.reserve_0 = sync_event.reserve_0;
            self.reserve_1 = sync_event.reserve_1;
            self.last_synced_log = new_log_index;

            Ok(())
        } else {
            Err(EventLogError::InvalidEventSignature)
        }
    }

    //Calculates base/quote, meaning the price of base token per quote (ie. exchange rate is X base per 1 quote)
    fn calculate_price(&self, base_token: H160) -> Result<f64, ArithmeticError> {
        Ok(q64_to_f64(self.calculate_price_64_x_64(base_token)?))
    }

    fn get_token_out(&self, token_in: H160) -> H160 {
        if self.token_a.address() == token_in {
            self.token_b.address()
        } else {
            self.token_a.address()
        }
    }

    fn simulate_swap(&self, token_in: H160, amount_in: U256) -> Result<U256, SwapSimulationError> {
        if self.token_a.address() == token_in {
            Ok(self.get_amount_out(
                amount_in,
                U256::from(self.reserve_0),
                U256::from(self.reserve_1),
            ))
        } else {
            Ok(self.get_amount_out(
                amount_in,
                U256::from(self.reserve_1),
                U256::from(self.reserve_0),
            ))
        }
    }

    fn simulate_swap_mut(&mut self, token_in: H160, amount_in: U256) -> Result<U256, SwapSimulationError> {
        if self.token_a.address() == token_in {
            let amount_out = self.get_amount_out(
                amount_in,
                U256::from(self.reserve_0),
                U256::from(self.reserve_1),
            );

            tracing::trace!(?amount_out);
            tracing::trace!(?self.reserve_0, ?self.reserve_1, "pool reserves before");

            self.reserve_0 += amount_in.as_u128();
            self.reserve_1 -= amount_out.as_u128();

            tracing::trace!(?self.reserve_0, ?self.reserve_1, "pool reserves after");

            Ok(amount_out)
        } else {
            let amount_out = self.get_amount_out(
                amount_in,
                U256::from(self.reserve_1),
                U256::from(self.reserve_0),
            );

            tracing::trace!(?amount_out);
            tracing::trace!(?self.reserve_0, ?self.reserve_1, "pool reserves before");

            self.reserve_0 -= amount_out.as_u128();
            self.reserve_1 += amount_in.as_u128();

            tracing::trace!(?self.reserve_0, ?self.reserve_1, "pool reserves after");

            Ok(amount_out)
        }
    }
}

impl UniswapV2Pool {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        address: H160,
        token_a: Currency,
        token_b: Currency,
        reserve_0: u128,
        reserve_1: u128,
        last_synced_log: (u64, u64),
        fee: u32,
    ) -> Self {
        Self {
            address,
            token_a,
            token_b,
            reserve_0,
            reserve_1,
            last_synced_log,
            fee,
        }
    }

    /// Creates a new instance of a the pool from a `PairCreated` event log.
    ///
    /// This method does not sync the pool data.
    pub fn new_empty_pool_from_log(log: Log) -> Result<Self, EventLogError> {
        let event_signature = log.topics[0];

        if event_signature == PAIR_CREATED_EVENT_SIGNATURE {
            let log_index = (
                log.block_number.unwrap().as_u64(),
                log.log_index.unwrap().as_u64(),
            );
            let pair_created_event = factory::PairCreatedFilter::decode_log(&RawLog::from(log))?;

            Ok(UniswapV2Pool {
                address: pair_created_event.pair,
                token_a: pair_created_event.token_0.into(),
                token_b: pair_created_event.token_1.into(),
                last_synced_log: log_index,
                ..Default::default()
            })
        } else {
            Err(EventLogError::InvalidEventSignature)?
        }
    }

    /// Returns the swap fee of the pool.
    pub fn fee(&self) -> u32 {
        self.fee
    }

    /// Returns whether the pool data is populated.
    pub fn data_is_populated(&self) -> bool {
        self.token_a.data_is_populated()
            && self.token_b.data_is_populated()
            && self.reserve_0 != 0
            && self.reserve_1 != 0
    }

    /// Calculates the price of the base token in terms of the quote token.
    ///
    /// Returned as a Q64 fixed point number.
    pub fn calculate_price_64_x_64(&self, base_token: H160) -> Result<u128, ArithmeticError> {
        let decimal_shift = self.token_a.decimals() as i8 - self.token_b.decimals() as i8;

        let (r_0, r_1) = if decimal_shift < 0 {
            (
                U256::from(self.reserve_0)
                    * U256::from(10u128.pow(decimal_shift.unsigned_abs() as u32)),
                U256::from(self.reserve_1),
            )
        } else {
            (
                U256::from(self.reserve_0),
                U256::from(self.reserve_1) * U256::from(10u128.pow(decimal_shift as u32)),
            )
        };

        if base_token == self.token_a.address() {
            if r_0.is_zero() {
                Ok(U128_0X10000000000000000)
            } else {
                div_uu(r_1, r_0)
            }
        } else if r_1.is_zero() {
            Ok(U128_0X10000000000000000)
        } else {
            div_uu(r_0, r_1)
        }
    }

    /// Calculates the amount received for a given `amount_in` `reserve_in` and `reserve_out`.
    pub fn get_amount_out(&self, amount_in: U256, reserve_in: U256, reserve_out: U256) -> U256 {
        tracing::trace!(?amount_in, ?reserve_in, ?reserve_out);

        if amount_in.is_zero() || reserve_in.is_zero() || reserve_out.is_zero() {
            return U256::zero();
        }
        let fee = (10000 - (self.fee / 10)) / 10; //Fee of 300 => (10,000 - 30) / 10  = 997
        let amount_in_with_fee = amount_in * U256::from(fee);
        let numerator = amount_in_with_fee * reserve_out;
        let denominator = reserve_in * U256::from(1000) + amount_in_with_fee;

        tracing::trace!(?fee, ?amount_in_with_fee, ?numerator, ?denominator);

        numerator / denominator
    }

    /// Returns the calldata for a swap.
    pub fn swap_calldata(
        &self,
        amount_0_out: U256,
        amount_1_out: U256,
        to: H160,
        calldata: Vec<u8>,
    ) -> Result<Bytes, ethers::abi::Error> {
        let input_tokens = vec![
            Token::Uint(amount_0_out),
            Token::Uint(amount_1_out),
            Token::Address(to),
            Token::Bytes(calldata),
        ];

        IUNISWAPV2PAIR_ABI
            .function("swap")?
            .encode_input(&input_tokens)
    }
}

pub fn div_uu(x: U256, y: U256) -> Result<u128, ArithmeticError> {
    let x = Uint::from_limbs(x.0);
    let y = Uint::from_limbs(y.0);
    if !y.is_zero() {
        let mut answer;

        if x <= U256_0XFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF {
            answer = (x << U256_64) / y;
        } else {
            let mut msb = U256_192;
            let mut xc = x >> U256_192;

            if xc >= U256_0X100000000 {
                xc >>= U256_32;
                msb += U256_32;
            }

            if xc >= U256_0X10000 {
                xc >>= U256_16;
                msb += U256_16;
            }

            if xc >= U256_0X100 {
                xc >>= U256_8;
                msb += U256_8;
            }

            if xc >= U256_16 {
                xc >>= U256_4;
                msb += U256_4;
            }

            if xc >= U256_4 {
                xc >>= U256_2;
                msb += U256_2;
            }

            if xc >= U256_2 {
                msb += U256_1;
            }

            answer = (x << (U256_255 - msb)) / (((y - U256_1) >> (msb - U256_191)) + U256_1);
        }

        if answer > U256_0XFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF {
            return Ok(0);
        }

        let hi = answer * (y >> U256_128);
        let mut lo = answer * (y & U256_0XFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF);

        let mut xh = x >> U256_192;
        let mut xl = x << U256_64;

        if xl < lo {
            xh -= U256_1;
        }

        xl = xl.overflowing_sub(lo).0;
        lo = hi << U256_128;

        if xl < lo {
            xh -= U256_1;
        }

        xl = xl.overflowing_sub(lo).0;

        if xh != hi >> U256_128 {
            return Err(ArithmeticError::RoundingError);
        }

        answer += xl / y;

        if answer > U256_0XFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF {
            return Ok(0_u128);
        }

        Ok(U256(answer.into_limbs()).as_u128())
    } else {
        Err(ArithmeticError::YIsZero)
    }
}

//Converts a Q64 fixed point to a Q16 fixed point -> f64
pub fn q64_to_f64(x: u128) -> f64 {
    BigFloat::from(x)
        .div(&BigFloat::from(U128_0X10000000000000000))
        .to_f64()
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use ethers::types::{H160, U256};

    use crate::amm::AutomatedMarketMaker;

    use super::UniswapV2Pool;

    #[test]
    fn test_swap_calldata() -> eyre::Result<()> {
        let uniswap_v2_pool = UniswapV2Pool::default();

        let _calldata = uniswap_v2_pool.swap_calldata(
            U256::from(123456789),
            U256::zero(),
            H160::from_str("0x41c36f504BE664982e7519480409Caf36EE4f008")?,
            vec![],
        );

        Ok(())
    }

    #[test]
    fn test_calculate_price_edge_case() -> eyre::Result<()> {
        let token_a = H160::from_str("0x0d500b1d8e8ef31e21c99d1db9a6444d3adf1270")?;
        let token_b = H160::from_str("0x8f18dc399594b451eda8c5da02d0563c0b2d0f16")?;
        let x = UniswapV2Pool {
            address: H160::from_str("0x652a7b75c229850714d4a11e856052aac3e9b065")?,
            token_a: token_a.into(),
            token_b: token_b.into(),
            reserve_0: 23595096345912178729927,
            reserve_1: 154664232014390554564,
            last_synced_log: (0, 0),
            fee: 300,
        };

        assert_ne!(x.calculate_price(token_a)?, 0.0);
        assert_ne!(x.calculate_price(token_b)?, 0.0);

        Ok(())
    }

    #[tokio::test]
    async fn test_calculate_price() -> eyre::Result<()> {
        // let rpc_endpoint = std::env::var("ETHEREUM_RPC_ENDPOINT")?;
        // let middleware = Arc::new(Provider::<Http>::try_from(rpc_endpoint)?);
        //
        // let mut pool = UniswapV2Pool {
        //     address: H160::from_str("0xB4e16d0168e52d35CaCD2c6185b44281Ec28C9Dc")?,
        //     ..Default::default()
        // };
        //
        // pool.populate_data(None, middleware.clone()).await?;
        //
        // pool.reserve_0 = 47092140895915;
        // pool.reserve_1 = 28396598565590008529300;
        //
        // let price_a_64_x = pool.calculate_price(pool.token_a.address())?;
        //
        // let price_b_64_x = pool.calculate_price(pool.token_b.address())?;
        //
        // assert_eq!(1658.3725965327264, price_b_64_x); //No precision loss: 30591574867092394336528 / 2**64
        // assert_eq!(0.0006030007985483893, price_a_64_x); //Precision loss: 11123401407064628 / 2**64
        //
        Ok(())
    }

    #[tokio::test]
    async fn test_calculate_price_64_x_64() -> eyre::Result<()> {
        // let rpc_endpoint = std::env::var("ETHEREUM_RPC_ENDPOINT")?;
        // let middleware = Arc::new(Provider::<Http>::try_from(rpc_endpoint)?);
        //
        // let mut pool = UniswapV2Pool {
        //     address: H160::from_str("0xB4e16d0168e52d35CaCD2c6185b44281Ec28C9Dc")?,
        //     ..Default::default()
        // };
        //
        // pool.populate_data(None, middleware.clone()).await?;
        //
        // pool.reserve_0 = 47092140895915;
        // pool.reserve_1 = 28396598565590008529300;
        //
        // let price_a_64_x = pool.calculate_price_64_x_64(pool.token_a.address())?;
        //
        // let price_b_64_x = pool.calculate_price_64_x_64(pool.token_b.address())?;
        //
        // assert_eq!(30591574867092394336528, price_b_64_x);
        // assert_eq!(11123401407064628, price_a_64_x);
        //
        Ok(())
    }
}
