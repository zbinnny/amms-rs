use std::hash::{Hash, Hasher};

use async_trait::async_trait;
use ethers::prelude::abigen;
use ethers::{
    abi::RawLog,
    prelude::EthEvent,
    types::{Log, H160, H256},
};
use serde::{Deserialize, Serialize};

use crate::amm::{factory::AutomatedMarketMakerFactory, AMM};

use super::UniswapV2Pool;

abigen!(
    IUniswapV2Factory,
    r#"[
        function getPair(address tokenA, address tokenB) external view returns (address pair)
        function allPairs(uint256 index) external view returns (address)
        event PairCreated(address indexed token0, address indexed token1, address pair, uint256)
        function allPairsLength() external view returns (uint256)

    ]"#;
);

pub const PAIR_CREATED_EVENT_SIGNATURE: H256 = H256([
    13, 54, 72, 189, 15, 107, 168, 1, 52, 163, 59, 169, 39, 90, 197, 133, 217, 211, 21, 240, 173,
    131, 85, 205, 222, 253, 227, 26, 250, 40, 208, 233,
]);

#[derive(Default, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UniswapV2Factory {
    pub address: H160,
    pub creation_block: u64,
    pub fee: u32,
}

impl UniswapV2Factory {
    pub fn new(address: H160, creation_block: u64, fee: u32) -> UniswapV2Factory {
        UniswapV2Factory {
            address,
            creation_block,
            fee,
        }
    }
}

impl Hash for UniswapV2Factory {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.address.hash(state);
    }
}

#[async_trait]
impl AutomatedMarketMakerFactory for UniswapV2Factory {
    fn address(&self) -> H160 {
        self.address
    }

    fn creation_block(&self) -> u64 {
        self.creation_block
    }

    fn amm_created_event_signature(&self) -> H256 {
        PAIR_CREATED_EVENT_SIGNATURE
    }

    fn new_empty_amm_from_log(&self, log: Log) -> Result<AMM, ethers::abi::Error> {
        let pair_created_event = PairCreatedFilter::decode_log(&RawLog::from(log))?;

        Ok(AMM::UniswapV2Pool(UniswapV2Pool {
            address: pair_created_event.pair,
            token_a: pair_created_event.token_0.into(),
            token_b: pair_created_event.token_1.into(),
            fee: self.fee,
            ..Default::default()
        }))
    }
}
