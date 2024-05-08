use std::{cmp::Ordering, sync::Arc};

use async_trait::async_trait;
use ethers::{
    abi::RawLog,
    prelude::EthEvent,
    providers::Middleware,
    types::{H160, H256, Log, U256},
};
use ethers::prelude::abigen;
use serde::{Deserialize, Serialize};
use tracing::instrument;

use crate::{
    amm::AutomatedMarketMaker,
    errors::{AMMError, ArithmeticError, EventLogError, SwapSimulationError},
};

use super::uniswap_v2::{div_uu, q64_to_f64, U128_0X10000000000000000};

pub mod batch_request;

abigen!(
    IERC4626Vault,
    r#"[
        function totalAssets() external view returns (uint256)
        function totalSupply() external view returns (uint256)
        function decimals() external view returns (uint8)
        event Withdraw(address indexed sender, address indexed receiver, address indexed owner, uint256 assets, uint256 shares)
        event Deposit(address indexed sender,address indexed owner, uint256 assets, uint256 shares)

    ]"#;
);

pub const DEPOSIT_EVENT_SIGNATURE: H256 = H256([
    220, 188, 28, 5, 36, 15, 49, 255, 58, 208, 103, 239, 30, 227, 92, 228, 153, 119, 98, 117, 46,
    58, 9, 82, 132, 117, 69, 68, 244, 199, 9, 215,
]);

pub const WITHDRAW_EVENT_SIGNATURE: H256 = H256([
    251, 222, 121, 125, 32, 28, 104, 27, 145, 5, 101, 41, 17, 158, 11, 2, 64, 124, 123, 185, 106,
    74, 44, 117, 192, 31, 201, 102, 114, 50, 200, 219,
]);

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ERC4626Vault {
    // token received from depositing, i.e. shares token
    pub vault_token: H160,
    pub vault_token_decimals: u8,
    // token received from withdrawing, i.e. underlying token
    pub asset_token: H160,
    pub asset_token_decimals: u8,
    // total supply of vault tokens
    pub vault_reserve: U256,
    // total balance of asset tokens held by vault
    pub asset_reserve: U256,
    // deposit fee in basis points
    pub deposit_fee: u32,
    // withdrawal fee in basis points
    pub withdraw_fee: u32,
    pub last_synced: (u64, u64),
}

#[async_trait]
impl AutomatedMarketMaker for ERC4626Vault {
    fn address(&self) -> H160 {
        self.vault_token
    }

    fn tokens(&self) -> Vec<H160> {
        vec![self.vault_token, self.asset_token]
    }

    fn last_synced_log(&self) -> (u64, u64) {
        self.last_synced
    }

    fn sync_on_event_signatures(&self) -> Vec<H256> {
        vec![DEPOSIT_EVENT_SIGNATURE, WITHDRAW_EVENT_SIGNATURE]
    }

    #[instrument(skip(self, middleware), level = "debug")]
    async fn sync<M: Middleware>(&mut self, middleware: Arc<M>) -> Result<(), AMMError<M>> {
        let (vault_reserve, asset_reserve) = self.get_reserves(middleware).await?;
        tracing::debug!(vault_reserve = ?vault_reserve, asset_reserve = ?asset_reserve, address = ?self.vault_token, "ER4626 sync");

        self.vault_reserve = vault_reserve;
        self.asset_reserve = asset_reserve;

        Ok(())
    }

    #[instrument(skip(self), level = "debug")]
    fn sync_from_log(&mut self, log: Log) -> Result<(), EventLogError> {
        let event_signature = log.topics[0];

        let block_number = log.block_number.clone().ok_or(EventLogError::LogBlockNumberNotFound)?.as_u64();
        let log_index = log.log_index.clone().ok_or(EventLogError::LogIndexNotFound)?.as_u64();

        if (block_number, log_index) <= self.last_synced {
            return Err(EventLogError::LogAlreadySynced);
        }

        if event_signature == DEPOSIT_EVENT_SIGNATURE {
            let deposit_event = DepositFilter::decode_log(&RawLog::from(log))?;
            self.asset_reserve += deposit_event.assets;
            self.vault_reserve += deposit_event.shares;
            tracing::debug!(asset_reserve = ?self.asset_reserve, vault_reserve = ?self.vault_reserve, address = ?self.vault_token, "ER4626 deposit event");
        } else if event_signature == WITHDRAW_EVENT_SIGNATURE {
            let withdraw_filter = WithdrawFilter::decode_log(&RawLog::from(log))?;
            self.asset_reserve -= withdraw_filter.assets;
            self.vault_reserve -= withdraw_filter.shares;
            tracing::debug!(asset_reserve = ?self.asset_reserve, vault_reserve = ?self.vault_reserve, address = ?self.vault_token, "ER4626 withdraw event");
        } else {
            return Err(EventLogError::InvalidEventSignature);
        }

        self.last_synced = (block_number, log_index);

        Ok(())
    }

    #[instrument(skip(self, middleware), level = "debug")]
    async fn populate_data<M: Middleware>(
        &mut self,
        _block_number: Option<u64>,
        middleware: Arc<M>,
    ) -> Result<(), AMMError<M>> {
        batch_request::get_4626_vault_data_batch_request(self, middleware.clone()).await?;

        Ok(())
    }

    fn calculate_price(&self, base_token: H160) -> Result<f64, ArithmeticError> {
        Ok(q64_to_f64(self.calculate_price_64_x_64(base_token)?))
    }

    fn get_token_out(&self, token_in: H160) -> H160 {
        if self.vault_token == token_in {
            self.asset_token
        } else {
            self.vault_token
        }
    }

    fn simulate_swap(&self, token_in: H160, amount_in: U256) -> Result<U256, SwapSimulationError> {
        if self.vault_token == token_in {
            Ok(self.get_amount_out(amount_in, self.vault_reserve, self.asset_reserve))
        } else {
            Ok(self.get_amount_out(amount_in, self.asset_reserve, self.vault_reserve))
        }
    }

    fn simulate_swap_mut(&mut self, token_in: H160, amount_in: U256) -> Result<U256, SwapSimulationError> {
        if self.vault_token == token_in {
            let amount_out = self.get_amount_out(amount_in, self.vault_reserve, self.asset_reserve);

            self.vault_reserve -= amount_in;
            self.asset_reserve -= amount_out;

            Ok(amount_out)
        } else {
            let amount_out = self.get_amount_out(amount_in, self.asset_reserve, self.vault_reserve);

            self.asset_reserve += amount_in;
            self.vault_reserve += amount_out;

            Ok(amount_out)
        }
    }
}

impl ERC4626Vault {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        vault_token: H160,
        vault_token_decimals: u8,
        asset_token: H160,
        asset_token_decimals: u8,
        vault_reserve: U256,
        asset_reserve: U256,
        deposit_fee: u32,
        withdraw_fee: u32,
        last_synced: (u64, u64),
    ) -> ERC4626Vault {
        ERC4626Vault {
            vault_token,
            vault_token_decimals,
            asset_token,
            asset_token_decimals,
            vault_reserve,
            asset_reserve,
            deposit_fee,
            withdraw_fee,
            last_synced,
        }
    }

    pub async fn new_from_address<M: Middleware>(vault_token: H160, middleware: Arc<M>) -> Result<Self, AMMError<M>> {
        let mut vault = ERC4626Vault {
            vault_token,
            ..Default::default()
        };

        vault.populate_data(None, middleware.clone()).await?;

        if !vault.data_is_populated() {
            return Err(AMMError::PoolDataError);
        }

        Ok(vault)
    }

    pub fn data_is_populated(&self) -> bool {
        !(self.vault_token.is_zero()
            || self.asset_token.is_zero()
            || self.vault_reserve.is_zero()
            || self.asset_reserve.is_zero())
    }

    pub async fn get_reserves<M: Middleware>(
        &self,
        middleware: Arc<M>,
    ) -> Result<(U256, U256), AMMError<M>> {
        //Initialize a new instance of the vault
        let vault = IERC4626Vault::new(self.vault_token, middleware);
        // Get the total assets in the vault
        let total_assets = match vault.total_assets().call().await {
            Ok(total_assets) => total_assets,
            Err(e) => return Err(AMMError::ContractError(e)),
        };
        // Get the total supply of the vault token
        let total_supply = match vault.total_supply().call().await {
            Ok(total_supply) => total_supply,
            Err(e) => return Err(AMMError::ContractError(e)),
        };

        Ok((total_supply, total_assets))
    }

    pub fn calculate_price_64_x_64(&self, base_token: H160) -> Result<u128, ArithmeticError> {
        let decimal_shift = self.vault_token_decimals as i8 - self.asset_token_decimals as i8;

        // Normalize reserves by decimal shift
        let (r_v, r_a) = match decimal_shift.cmp(&0) {
            Ordering::Less => (
                self.vault_reserve * U256::from(10u128.pow(decimal_shift.unsigned_abs() as u32)),
                self.asset_reserve,
            ),
            _ => (
                self.vault_reserve,
                self.asset_reserve * U256::from(10u128.pow(decimal_shift as u32)),
            ),
        };

        // Withdraw
        if base_token == self.vault_token {
            if r_v.is_zero() {
                // Return 1 in Q64
                Ok(U128_0X10000000000000000)
            } else {
                Ok(div_uu(r_a, r_v)?)
            }
            // Deposit
        } else if r_a.is_zero() {
            // Return 1 in Q64
            Ok(U128_0X10000000000000000)
        } else {
            Ok(div_uu(r_v, r_a)?)
        }
    }

    pub fn get_amount_out(&self, amount_in: U256, reserve_in: U256, reserve_out: U256) -> U256 {
        if amount_in.is_zero() {
            return U256::zero();
        }

        if self.vault_reserve.is_zero() {
            return amount_in;
        }

        let fee = if reserve_in == self.vault_reserve {
            self.withdraw_fee
        } else {
            self.deposit_fee
        };

        amount_in * reserve_out / reserve_in * (10000 - fee) / 10000
    }
}

#[cfg(test)]
mod tests {
    use std::{str::FromStr, sync::Arc};

    use ethers::{
        providers::{Http, Provider},
        types::{H160, U256},
    };

    use crate::amm::AutomatedMarketMaker;

    use super::ERC4626Vault;

    #[tokio::test]
    async fn test_get_vault_data() -> eyre::Result<()> {
        let rpc_endpoint = std::env::var("ETHEREUM_RPC_ENDPOINT")?;
        let middleware = Arc::new(Provider::<Http>::try_from(rpc_endpoint)?);

        let mut vault = ERC4626Vault {
            vault_token: H160::from_str("0x163538E22F4d38c1eb21B79939f3d2ee274198Ff")?,
            ..Default::default()
        };

        vault.populate_data(None, middleware).await?;

        assert_eq!(vault.vault_token_decimals, 18);
        assert_eq!(
            vault.asset_token,
            H160::from_str("0x6B175474E89094C44Da98b954EedeAC495271d0F")?
        );
        assert_eq!(vault.asset_token_decimals, 18);
        assert_eq!(vault.deposit_fee, 0);
        assert_eq!(vault.withdraw_fee, 0);

        Ok(())
    }

    #[tokio::test]
    async fn test_calculate_price_varying_decimals() -> eyre::Result<()> {
        let rpc_endpoint = std::env::var("ETHEREUM_RPC_ENDPOINT")?;
        let middleware = Arc::new(Provider::<Http>::try_from(rpc_endpoint)?);

        let mut vault = ERC4626Vault {
            vault_token: H160::from_str("0x163538E22F4d38c1eb21B79939f3d2ee274198Ff")?,
            ..Default::default()
        };

        vault.populate_data(None, middleware).await?;

        vault.vault_reserve = U256::from_dec_str("501910315708981197269904")?;
        vault.asset_token_decimals = 6;
        vault.asset_reserve = U256::from_dec_str("505434849031")?;

        let price_v_64_x = vault.calculate_price(vault.vault_token)?;
        let price_a_64_x = vault.calculate_price(vault.asset_token)?;

        assert_eq!(price_v_64_x, 1.0070222372637234);
        assert_eq!(price_a_64_x, 0.99302673068789);

        Ok(())
    }

    #[tokio::test]
    async fn test_calculate_price_zero_reserve() -> eyre::Result<()> {
        let rpc_endpoint = std::env::var("ETHEREUM_RPC_ENDPOINT")?;
        let middleware = Arc::new(Provider::<Http>::try_from(rpc_endpoint)?);

        let mut vault = ERC4626Vault {
            vault_token: H160::from_str("0x163538E22F4d38c1eb21B79939f3d2ee274198Ff")?,
            ..Default::default()
        };

        vault.populate_data(None, middleware).await?;

        vault.vault_reserve = U256::from_dec_str("0")?;
        vault.asset_reserve = U256::from_dec_str("0")?;

        let price_v_64_x = vault.calculate_price(vault.vault_token)?;
        let price_a_64_x = vault.calculate_price(vault.asset_token)?;

        assert_eq!(price_v_64_x, 1.0);
        assert_eq!(price_a_64_x, 1.0);

        Ok(())
    }

    #[tokio::test]
    async fn test_calculate_price() -> eyre::Result<()> {
        let rpc_endpoint = std::env::var("ETHEREUM_RPC_ENDPOINT")?;
        let middleware = Arc::new(Provider::<Http>::try_from(rpc_endpoint)?);

        let mut vault = ERC4626Vault {
            vault_token: H160::from_str("0x163538E22F4d38c1eb21B79939f3d2ee274198Ff")?,
            ..Default::default()
        };

        vault.populate_data(None, middleware).await?;

        vault.vault_reserve = U256::from_dec_str("501910315708981197269904")?;
        vault.asset_reserve = U256::from_dec_str("505434849031054568651911")?;

        let price_v_64_x = vault.calculate_price(vault.vault_token)?;
        let price_a_64_x = vault.calculate_price(vault.asset_token)?;

        assert_eq!(price_v_64_x, 1.0070222372638322);
        assert_eq!(price_a_64_x, 0.9930267306877828);

        Ok(())
    }

    #[tokio::test]
    async fn test_calculate_price_64_x_64() -> eyre::Result<()> {
        let rpc_endpoint = std::env::var("ETHEREUM_RPC_ENDPOINT")?;
        let middleware = Arc::new(Provider::<Http>::try_from(rpc_endpoint)?);

        let mut vault = ERC4626Vault {
            vault_token: H160::from_str("0x163538E22F4d38c1eb21B79939f3d2ee274198Ff")?,
            ..Default::default()
        };

        vault.populate_data(None, middleware).await?;

        vault.vault_reserve = U256::from_dec_str("501910315708981197269904")?;
        vault.asset_reserve = U256::from_dec_str("505434849031054568651911")?;

        let price_v_64_x = vault.calculate_price_64_x_64(vault.vault_token)?;
        let price_a_64_x = vault.calculate_price_64_x_64(vault.asset_token)?;

        assert_eq!(price_v_64_x, 18576281487340329878);
        assert_eq!(price_a_64_x, 18318109959350028841);

        Ok(())
    }

    #[tokio::test]
    async fn test_simulate_swap() -> eyre::Result<()> {
        let rpc_endpoint = std::env::var("ETHEREUM_RPC_ENDPOINT")?;
        let middleware = Arc::new(Provider::<Http>::try_from(rpc_endpoint)?);

        let mut vault = ERC4626Vault {
            vault_token: H160::from_str("0x163538E22F4d38c1eb21B79939f3d2ee274198Ff")?,
            ..Default::default()
        };

        vault.populate_data(None, middleware).await?;

        vault.vault_reserve = U256::from_dec_str("501910315708981197269904")?;
        vault.asset_reserve = U256::from_dec_str("505434849031054568651911")?;

        let assets_out = vault.simulate_swap(
            vault.vault_token,
            U256::from_dec_str("3000000000000000000")?,
        )?;
        let shares_out = vault.simulate_swap(
            vault.asset_token,
            U256::from_dec_str("3000000000000000000")?,
        )?;

        assert_eq!(assets_out, U256::from_dec_str("3021066711791496478")?);
        assert_eq!(shares_out, U256::from_dec_str("2979080192063348487")?);

        Ok(())
    }
}
