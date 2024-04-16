use {
    solana_program_runtime::declare_process_instruction,
    solana_sdk::compute_budget::DEFAULT_COMPUTE_UNITS,
};

declare_process_instruction!(Entrypoint, DEFAULT_COMPUTE_UNITS, |_invoke_context| {
    // Do nothing, compute budget instructions handled by the runtime
    Ok(())
});
