use {
    solana_builtins_default_costs::BUILTIN_INSTRUCTION_COSTS,
    solana_sdk::{
        instruction::CompiledInstruction, pubkey::Pubkey, saturating_add_assign,
        transaction::Result,
    },
};

#[cfg_attr(test, derive(Clone, Eq, PartialEq))]
#[derive(Default, Debug)]
pub struct BuiltinInstructionDetails {
    // builtin instruction details
    pub sum_builtin_compute_units: u32,
    pub count_builtin_instructions: u32,
    pub count_non_builtin_instructions: u32,
}

impl BuiltinInstructionDetails {
    pub fn process_instruction<'a>(
        &mut self,
        program_id: &'a Pubkey,
        _instruction: &'a CompiledInstruction, // reserved to identify builtin cost pby instruction
                                               // instead of by program_id
    ) -> Result<()> {
        if let Some(builtin_ix_cost) = BUILTIN_INSTRUCTION_COSTS.get(program_id) {
            saturating_add_assign!(
                self.sum_builtin_compute_units,
                u32::try_from(*builtin_ix_cost).unwrap()
            );
            saturating_add_assign!(self.count_builtin_instructions, 1);
        } else {
            saturating_add_assign!(self.count_non_builtin_instructions, 1);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_process_instruction() {
        let mut builtin_instruction_details = BuiltinInstructionDetails::default();
        let dummy_ix = CompiledInstruction::new_from_raw_parts(0, vec![], vec![]);

        // process all builtin with default costs
        for (pubkey, cost) in BUILTIN_INSTRUCTION_COSTS.iter() {
            let expected_value = BuiltinInstructionDetails {
                sum_builtin_compute_units: builtin_instruction_details.sum_builtin_compute_units
                    + *cost as u32,
                count_builtin_instructions: builtin_instruction_details.count_builtin_instructions
                    + 1,
                count_non_builtin_instructions: 0,
            };

            assert!(builtin_instruction_details
                .process_instruction(pubkey, &dummy_ix)
                .is_ok());
            assert_eq!(builtin_instruction_details, expected_value);
        }

        // continue process non-builtin instruction
        let expected_value = BuiltinInstructionDetails {
            sum_builtin_compute_units: builtin_instruction_details.sum_builtin_compute_units,
            count_builtin_instructions: builtin_instruction_details.count_builtin_instructions,
            count_non_builtin_instructions: builtin_instruction_details
                .count_non_builtin_instructions
                + 1,
        };

        assert!(builtin_instruction_details
            .process_instruction(&Pubkey::new_unique(), &dummy_ix)
            .is_ok());
        assert_eq!(builtin_instruction_details, expected_value);
    }
}
