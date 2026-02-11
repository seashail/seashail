// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

contract MockSwapRouter02 {
    struct ExactInputSingleParams {
        address tokenIn;
        address tokenOut;
        uint24 fee;
        address recipient;
        uint256 amountIn;
        uint256 amountOutMinimum;
        uint160 sqrtPriceLimitX96;
    }

    function exactInputSingle(
        ExactInputSingleParams calldata params
    ) external payable returns (uint256 amountOut) {
        // Deterministic "swap" for E2E testing.
        amountOut = params.amountIn * 2;
        (params.tokenIn, params.tokenOut, params.fee, params.recipient, params.amountOutMinimum, params.sqrtPriceLimitX96);
    }

    function multicall(bytes[] calldata data) external payable returns (bytes[] memory results) {
        results = new bytes[](data.length);
        for (uint256 i = 0; i < data.length; i++) {
            results[i] = data[i];
        }
    }

    function unwrapWETH9(uint256, address) external payable {}
    function refundETH() external payable {}
}
