// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

contract MockQuoterV2 {
    struct QuoteExactInputSingleParams {
        address tokenIn;
        address tokenOut;
        uint256 amountIn;
        uint24 fee;
        uint160 sqrtPriceLimitX96;
    }

    function quoteExactInputSingle(
        QuoteExactInputSingleParams calldata params
    ) external pure returns (uint256 amountOut, uint160, uint32, uint256) {
        // Deterministic "quote" for E2E testing.
        amountOut = params.amountIn * 2;
        return (amountOut, 0, 0, 0);
    }
}
