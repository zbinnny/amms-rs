//SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/**
 @dev This contract is not meant to be deployed. Instead, use a static call with the
      deployment bytecode as payload.
 */
contract GetTokenInfo {
    struct TokenInfo {
        string symbol;
        uint8 decimals;
    }

    constructor(address[] memory tokens) {
        TokenInfo[] memory tokenInfos = new TokenInfo[](tokens.length);

        for (uint256 i = 0; i < tokens.length; ++i) {
            address tokenAddress = tokens[i];
            if (tokenAddress.code.length == 0) continue;

            TokenInfo memory tokenInfo;

            // symbol
            (bool success, bytes memory data) = tokenAddress.staticcall(abi.encodeWithSignature("symbol()"));
            if (!success) continue;
            tokenInfo.symbol = abi.decode(data, (string));

            // decimals
            (success, data) = tokenAddress.staticcall(abi.encodeWithSignature("decimals()"));
            if (!success) continue;
            tokenInfo.decimals = abi.decode(data, (uint8));

            tokenInfos[i] = tokenInfo;
        }

        // ensure abi encoding, not needed here but increase reusability for different return types
        // note: abi.encode add a first 32 bytes word with the address of the original data
        bytes memory _abiEncodedData = abi.encode(tokenInfos);

        assembly {
        // Return from the start of the data (discarding the original data address)
        // up to the end of the memory used
            let dataStart := add(_abiEncodedData, 0x20)
            return (dataStart, sub(msize(), dataStart))
        }
    }
}
