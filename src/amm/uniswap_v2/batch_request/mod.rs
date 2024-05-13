use std::sync::Arc;

use ethers::prelude::abigen;
use ethers::{
    abi::{ParamType, Token},
    providers::Middleware,
    types::{Bytes, H160, U256},
};

use crate::errors::AMMError;

abigen!(

    IGetUniswapV2PairsBatchRequest,
        "src/amm/uniswap_v2/batch_request/GetUniswapV2PairsBatchRequestABI.json";

    IGetUniswapV2PoolDataBatchRequest,
        "src/amm/uniswap_v2/batch_request/GetUniswapV2PoolDataBatchRequestABI.json";
);

pub async fn get_pairs_batch_request<M: Middleware>(
    factory: H160,
    from: U256,
    step: U256,
    middleware: Arc<M>,
) -> Result<Vec<H160>, AMMError<M>> {
    let mut pairs = vec![];

    let constructor_args = Token::Tuple(vec![
        Token::Uint(from),
        Token::Uint(step),
        Token::Address(factory),
    ]);

    let deployer = IGetUniswapV2PairsBatchRequest::deploy(middleware, constructor_args)?;
    let return_data: Bytes = deployer.call_raw().await?;

    let return_data_tokens = ethers::abi::decode(
        &[ParamType::Array(Box::new(ParamType::Address))],
        &return_data,
    )?;

    for token_array in return_data_tokens {
        if let Some(arr) = token_array.into_array() {
            for token in arr {
                if let Some(addr) = token.into_address() {
                    if !addr.is_zero() {
                        pairs.push(addr);
                    }
                }
            }
        }
    }

    Ok(pairs)
}
