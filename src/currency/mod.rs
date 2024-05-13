use std::collections::HashSet;
use std::sync::Arc;

use abi::ParamType;
use ethers::abi;
use ethers::abi::Token;
use ethers::prelude::{abigen, Address, StreamExt};
use ethers::providers::Middleware;
use futures::stream::FuturesUnordered;
use serde::{Deserialize, Serialize};

use crate::errors::{AMMError, CurrencyError};

abigen!(IGetCurrencyInfo, "src/currency/GetTokenInfo.json",);

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Currency {
    address: Address,
    symbol: String,
    decimals: u8,
}

impl Currency {
    pub fn new<H: Into<Address>>(address: H) -> Self {
        Self {
            address: address.into(),
            ..Default::default()
        }
    }

    pub fn new_with_tokens<H: Into<Address>>(address: H, tokens: Vec<Token>) -> Self {
        let mut currency = Self::new(address);
        currency.apply_tokens(tokens);
        currency
    }

    fn apply_tokens(&mut self, tokens: Vec<Token>) {
        self.symbol = tokens[0].to_owned().into_string().unwrap();
        self.decimals = tokens[1].to_owned().into_uint().unwrap().as_u32() as u8;
    }

    pub fn is_invalid_token(&self) -> bool {
        self.address.is_zero() || self.symbol.len() == 0
    }

    pub fn data_is_filled(&self) -> bool {
        self.symbol.len() > 0
    }

    pub fn address(&self) -> Address {
        self.address
    }

    pub fn symbol(&self) -> String {
        self.symbol.clone()
    }

    pub fn decimals(&self) -> u8 {
        self.decimals
    }
}

impl<H> From<H> for Currency
where
    H: Into<Address>,
{
    fn from(address: H) -> Self {
        Self::new(address)
    }
}

pub async fn batch_get_currency_info<M: Middleware>(
    currencies: Vec<Address>,
    batch_size: Option<usize>,
    middleware: Arc<M>,
) -> Result<Vec<Currency>, AMMError<M>> {
    let currencies_set: HashSet<Address> = currencies.iter().cloned().collect();
    let currencies: Vec<Address> = currencies_set.into_iter().collect();
    let batch_size = batch_size.unwrap_or_else(|| 150);

    // 初始化并行任务
    let mut futures = FuturesUnordered::new();
    for chunk in currencies.chunks(batch_size) {
        let chunk = chunk.to_vec().clone();
        let middleware = middleware.clone();

        futures.push(async move { get_currencies(chunk, middleware).await });
    }

    // 收集结果
    let mut results = vec![];
    while let Some(result) = futures.next().await {
        match result {
            Ok(v) => results.extend(v),
            Err(_) => continue,
        }
    }

    Ok(results)
}

async fn get_currencies<M: Middleware>(
    currencies: Vec<Address>,
    middleware: Arc<M>,
) -> Result<Vec<Currency>, AMMError<M>> {
    let token_addresses: Vec<Token> = currencies
        .iter()
        .map(|currency| Token::Address(currency.clone()))
        .collect();

    let constructor_args = Token::Tuple(vec![Token::Array(token_addresses)]);

    let deployer = IGetCurrencyInfo::deploy(middleware.clone(), constructor_args)?;

    let return_data = match deployer.call_raw().await {
        Ok(data) => data,
        Err(err) => {
            tracing::error!(err = ?err, currencies = ?currencies, "get_currencies error");
            return Ok(vec![]);
        }
    };
    let return_data_tokens = abi::decode(
        &[ParamType::Array(Box::new(ParamType::Tuple(vec![
            ParamType::String,  // token symbol
            ParamType::Uint(8), // token decimals
        ])))],
        &return_data,
    )?;

    let return_data_tokens = return_data_tokens[0]
        .clone()
        .into_array()
        .ok_or(CurrencyError::InvalidReturnData)?;

    let mut currencies: Vec<Currency> = currencies.into_iter().map(|c| c.into()).collect();
    for index in 0..currencies.len() {
        // 解析 token info
        let tokens = match return_data_tokens[index].clone().into_tuple() {
            Some(tokens) => tokens.clone(),
            _ => continue,
        };

        currencies[index].apply_tokens(tokens);
    }

    Ok(currencies)
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;
    use std::sync::Arc;

    use ethers::prelude::{Address, Ws};
    use ethers::providers::Provider;
    use eyre::Result;
    use tracing::Level;
    use tracing_subscriber::FmtSubscriber;

    use crate::currency::batch_get_currency_info;

    pub fn init_tracing() {
        // 创建一个 FmtSubscriber 实例并设置打印等级
        let subscriber = FmtSubscriber::builder()
            .with_max_level(Level::DEBUG)
            .finish();

        // 全局注册 subscriber
        tracing::subscriber::set_global_default(subscriber)
            .expect("setting default subscriber failed");
    }

    #[tokio::test]
    async fn test_batch_get_currency_info() -> Result<()> {
        init_tracing();

        let provider = Provider::<Ws>::connect("wss://ethereum-rpc.publicnode.com").await?;
        let middleware = Arc::new(provider);

        let mut currencies: Vec<_> = vec![
            // H160::from_str("0xdAC17F958D2ee523a2206206994597C13D831ec7")?, // usdt
            // "0xdcf7afa9d41f4394ef9e5d52ef5eb2a23fc05234",
            // "0x64aa3364f17a4d01c6f1751fd97c2bd3d7e7f1d5",
            // "0x8abffcfca4c21ba0220fdd6754804268333b2fac",
            // "0x0eb7ef278e994e7c8fa280e1c54d683dd9b93fc5",
            // "0x9b2d81a1ae36e8e66a0875053429816f0b6b829e",

            // "0x6117d14fd9dba782397d13c4e9ae75e4ce066589",
            // "0xf68a4906b960c344865820bcb9b9d4d9ff93c9f9",
            "0x62eef4ec8d58ad37dcf17c0ead9b8b291d93b62c",
            // "0xac6f8d7c06658f1540c1b207203cebdd60ef71cf",
            // "0x48f7d215f70c804f178ae0fe9d5fac2a9e2a8e94",
        ]
        .into_iter()
        .map(|a| Address::from_str(a).unwrap())
        .collect();

        let result = batch_get_currency_info(currencies.clone(), Some(100), middleware)
            .await
            .unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].symbol, "USDT");
        assert_eq!(result[0].decimals, 6);

        for item in result.iter() {
            tracing::info!(?item);
        }

        Ok(())
    }
}
