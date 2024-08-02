use {
    crate::{builtin_instruction_details::*, compute_budget_instruction_details::*},
    solana_compute_budget::compute_budget_limits::*,
    solana_sdk::{
        instruction::CompiledInstruction,
        pubkey::Pubkey,
        transaction::{Result, TransactionError},
    },
    std::num::NonZeroU32,
};

/// Information about instructions gathered after scan over transaction;
/// These are "raw" information that suitable for cache and reuse.
#[cfg_attr(test, derive(Eq, PartialEq))]
#[derive(Default, Debug)]
pub struct InstructionDetails {
    builtin_instruction_details: BuiltinInstructionDetails,
    compute_budget_instruction_details: ComputeBudgetInstructionDetails,
}

impl InstructionDetails {
    pub fn try_from<'a>(
        instructions: impl Iterator<Item = (&'a Pubkey, &'a CompiledInstruction)>,
    ) -> Result<Self> {
        let mut compute_budget_instruction_details = ComputeBudgetInstructionDetails::default();
        let mut builtin_instruction_details = BuiltinInstructionDetails::default();

        for (i, (program_id, instruction)) in instructions.enumerate() {
            compute_budget_instruction_details.process_instruction(
                i as u8,
                program_id,
                instruction,
            )?;
            builtin_instruction_details.process_instruction(program_id, instruction)?;
        }

        Ok(Self {
            builtin_instruction_details,
            compute_budget_instruction_details,
        })
    }

    pub fn sanitize_and_convert_to_compute_budget_limits(&self) -> Result<ComputeBudgetLimits> {
        // Sanitize requested heap size
        let updated_heap_bytes = self
            .compute_budget_instruction_details
            .requested_heap_size
            .map_or(MIN_HEAP_FRAME_BYTES, |(_index, requested_heap_size)| {
                requested_heap_size
            })
            .min(MAX_HEAP_FRAME_BYTES);

        // Calculate compute unit limit
        let compute_unit_limit = self
            .compute_budget_instruction_details
            .requested_compute_unit_limit
            .map_or_else(
                || {
                    // NOTE: to match current behavior of:
                    // num_non_compute_budget_instructions * DEFAULT
                    self.builtin_instruction_details
                        .count_builtin_instructions
                        .saturating_add(
                            self.builtin_instruction_details
                                .count_non_builtin_instructions,
                        )
                        .saturating_sub(
                            self.compute_budget_instruction_details
                                .count_compute_budget_instructions,
                        )
                        .saturating_mul(DEFAULT_INSTRUCTION_COMPUTE_UNIT_LIMIT)
                },
                |(_index, requested_compute_unit_limit)| requested_compute_unit_limit,
            )
            .min(MAX_COMPUTE_UNIT_LIMIT);

        let compute_unit_price = self
            .compute_budget_instruction_details
            .requested_compute_unit_price
            .map_or(0, |(_index, requested_compute_unit_price)| {
                requested_compute_unit_price
            });

        let loaded_accounts_bytes =
            if let Some((_index, requested_loaded_accounts_data_size_limit)) = self
                .compute_budget_instruction_details
                .requested_loaded_accounts_data_size_limit
            {
                NonZeroU32::new(requested_loaded_accounts_data_size_limit)
                    .ok_or(TransactionError::InvalidLoadedAccountsDataSizeLimit)?
            } else {
                MAX_LOADED_ACCOUNTS_DATA_SIZE_BYTES
            }
            .min(MAX_LOADED_ACCOUNTS_DATA_SIZE_BYTES);

        Ok(ComputeBudgetLimits {
            updated_heap_bytes,
            compute_unit_limit,
            compute_unit_price,
            loaded_accounts_bytes,
        })
    }
}

#[cfg(test)]
mod test {
    use {
        super::*,
        solana_sdk::{
            compute_budget::ComputeBudgetInstruction,
            instruction::{Instruction, InstructionError},
            message::Message,
            pubkey::Pubkey,
            signature::Keypair,
            signer::Signer,
            system_instruction::{self},
            transaction::{SanitizedTransaction, Transaction},
        },
    };

    macro_rules! test {
        ( $payer_keypair: expr, $instructions: expr, $expected_result: expr) => {
            let tx = SanitizedTransaction::from_transaction_for_tests(Transaction::new_unsigned(
                Message::new($instructions, Some(&$payer_keypair.pubkey())),
            ));

            let result = InstructionDetails::try_from(tx.message().program_instructions_iter());
            assert_eq!($expected_result, result);
        };
    }

    #[test]
    fn test_try_from() {
        let payer_keypair = Keypair::new();
        test!(payer_keypair, &[], Ok(InstructionDetails::default()));

        test!(
            payer_keypair,
            &[
                Instruction::new_with_bincode(Pubkey::new_unique(), &0_u8, vec![]),
                ComputeBudgetInstruction::request_heap_frame(40 * 1024),
                ComputeBudgetInstruction::set_compute_unit_limit(u32::MAX),
                ComputeBudgetInstruction::set_compute_unit_price(u64::MAX),
                ComputeBudgetInstruction::set_loaded_accounts_data_size_limit(u32::MAX),
                Instruction::new_with_bincode(Pubkey::new_unique(), &0_u8, vec![]),
                system_instruction::transfer(&payer_keypair.pubkey(), &Pubkey::new_unique(), 1),
            ],
            Ok(InstructionDetails {
                builtin_instruction_details: BuiltinInstructionDetails {
                    sum_builtin_compute_units: 4
                        * solana_compute_budget_program::DEFAULT_COMPUTE_UNITS as u32
                        + solana_system_program::system_processor::DEFAULT_COMPUTE_UNITS as u32,
                    count_builtin_instructions: 5,
                    count_non_builtin_instructions: 2,
                },
                compute_budget_instruction_details: ComputeBudgetInstructionDetails {
                    requested_compute_unit_limit: Some((2, u32::MAX)),
                    requested_compute_unit_price: Some((3, u64::MAX)),
                    requested_heap_size: Some((1, 40 * 1024)),
                    requested_loaded_accounts_data_size_limit: Some((4, u32::MAX)),
                    count_compute_budget_instructions: 4,
                },
            })
        );

        // any invalid intruction would error out
        test!(
            payer_keypair,
            &[
                Instruction::new_with_bincode(Pubkey::new_unique(), &0_u8, vec![]),
                ComputeBudgetInstruction::request_heap_frame(0), // invalid
                Instruction::new_with_bincode(Pubkey::new_unique(), &0_u8, vec![]),
            ],
            Err(TransactionError::InstructionError(
                1,
                InstructionError::InvalidInstructionData,
            ))
        );
    }

    #[test]
    fn test_sanitize_and_convert_to_compute_budget_limits() {
        let instruction_details = InstructionDetails::default();
        assert_eq!(
            instruction_details.sanitize_and_convert_to_compute_budget_limits(),
            Ok(ComputeBudgetLimits {
                compute_unit_limit: 0,
                ..ComputeBudgetLimits::default()
            })
        );

        let count_builtin_instructions = 5;
        let count_non_builtin_instructions = 2;
        let count_compute_budget_instructions = 4;
        let builtin_instruction_details = BuiltinInstructionDetails {
            count_builtin_instructions,
            count_non_builtin_instructions,
            ..BuiltinInstructionDetails::default()
        };

        // no compute-budget instructions, all default ComputeBudgetLimits except cu-limit
        let instruction_details = InstructionDetails {
            builtin_instruction_details: builtin_instruction_details.clone(),
            ..InstructionDetails::default()
        };
        let expected_compute_unit_limit = (count_builtin_instructions
            + count_non_builtin_instructions)
            * DEFAULT_INSTRUCTION_COMPUTE_UNIT_LIMIT;
        assert_eq!(
            instruction_details.sanitize_and_convert_to_compute_budget_limits(),
            Ok(ComputeBudgetLimits {
                compute_unit_limit: expected_compute_unit_limit,
                ..ComputeBudgetLimits::default()
            })
        );

        let instruction_details = InstructionDetails {
            builtin_instruction_details: builtin_instruction_details.clone(),
            compute_budget_instruction_details: ComputeBudgetInstructionDetails {
                requested_compute_unit_limit: Some((1, 0)),
                requested_compute_unit_price: Some((2, 0)),
                requested_heap_size: Some((3, 0)),
                requested_loaded_accounts_data_size_limit: Some((4, 0)),
                count_compute_budget_instructions,
            },
        };
        assert_eq!(
            instruction_details.sanitize_and_convert_to_compute_budget_limits(),
            Err(TransactionError::InvalidLoadedAccountsDataSizeLimit)
        );

        let instruction_details = InstructionDetails {
            builtin_instruction_details: builtin_instruction_details.clone(),
            compute_budget_instruction_details: ComputeBudgetInstructionDetails {
                requested_compute_unit_limit: Some((1, u32::MAX)),
                requested_compute_unit_price: Some((2, u64::MAX)),
                requested_heap_size: Some((3, u32::MAX)),
                requested_loaded_accounts_data_size_limit: Some((4, u32::MAX)),
                count_compute_budget_instructions,
            },
        };
        assert_eq!(
            instruction_details.sanitize_and_convert_to_compute_budget_limits(),
            Ok(ComputeBudgetLimits {
                updated_heap_bytes: MAX_HEAP_FRAME_BYTES,
                compute_unit_limit: MAX_COMPUTE_UNIT_LIMIT,
                compute_unit_price: u64::MAX,
                loaded_accounts_bytes: MAX_LOADED_ACCOUNTS_DATA_SIZE_BYTES,
            })
        );

        let val: u32 = 1024;
        let instruction_details = InstructionDetails {
            builtin_instruction_details,
            compute_budget_instruction_details: ComputeBudgetInstructionDetails {
                requested_compute_unit_limit: Some((1, val)),
                requested_compute_unit_price: Some((2, val as u64)),
                requested_heap_size: Some((3, val)),
                requested_loaded_accounts_data_size_limit: Some((4, val)),
                count_compute_budget_instructions,
            },
        };
        assert_eq!(
            instruction_details.sanitize_and_convert_to_compute_budget_limits(),
            Ok(ComputeBudgetLimits {
                updated_heap_bytes: val,
                compute_unit_limit: val,
                compute_unit_price: val as u64,
                loaded_accounts_bytes: NonZeroU32::new(val).unwrap(),
            })
        );
    }
}
