// static account keys has max
use {
    agave_transaction_view::static_account_keys_frame::MAX_STATIC_ACCOUNTS_PER_PACKET as FILTER_SIZE,
    solana_builtins_default_costs::{get_builtin_instruction_cost, MAYBE_BUILTIN_KEY},
    solana_sdk::{feature_set::FeatureSet, pubkey::Pubkey},
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ProgramKind {
    NotBuiltin,
    Builtin { is_compute_budget: bool },
}

pub(crate) struct ComputeBudgetProgramIdFilter {
    // array of slots for all possible static and sanitized program_id_index,
    // each slot indicates if a program_id_index has not been checked (eg, None),
    // or already checked with result (eg, Some(ProgramKind)) that can be reused.
    program_kind: [Option<ProgramKind>; FILTER_SIZE as usize],
}

impl ComputeBudgetProgramIdFilter {
    pub(crate) fn new() -> Self {
        ComputeBudgetProgramIdFilter {
            program_kind: [None; FILTER_SIZE as usize],
        }
    }

    #[inline]
    pub(crate) fn get_program_kind(&mut self, index: usize, program_id: &Pubkey, feature_set: &FeatureSet) -> ProgramKind {
        *self
            .program_kind
            .get_mut(index)
            .expect("program id index is sanitized")
            .get_or_insert_with(|| Self::check_program_kind(program_id, feature_set))
    }

    #[inline]
    fn check_program_kind(program_id: &Pubkey, feature_set: &FeatureSet) -> ProgramKind {
        if !MAYBE_BUILTIN_KEY[program_id.as_ref()[0] as usize] {
            return ProgramKind::NotBuiltin;
        }

        get_builtin_instruction_cost(
            program_id, feature_set
        )
        .map_or(ProgramKind::NotBuiltin, |_default_cost| {
            ProgramKind::Builtin {
                is_compute_budget: solana_sdk::compute_budget::check_id(program_id),
            }
        })
    }
}

#[cfg(test)]
mod test {
    use super::*;

    const DUMMY_PROGRAM_ID: &str = "dummmy1111111111111111111111111111111111111";

    fn get_program_kind(feature_set: &FeatureSet) {
        let mut test_store = ComputeBudgetProgramIdFilter::new();
        let mut index = 9;

        // initial state is Unchecked
        assert!(test_store.program_kind[index].is_none());

        // non builtin returns None
        assert_eq!(
            test_store.get_program_kind(index, &DUMMY_PROGRAM_ID.parse().unwrap(), &feature_set),
            ProgramKind::NotBuiltin
        );
        // but its state is now checked (eg, Some(...))
        assert_eq!(
            test_store.program_kind[index],
            Some(ProgramKind::NotBuiltin)
        );
        // lookup same `index` will return cached data, will not lookup `program_id`
        // again
        assert_eq!(
            test_store.get_program_kind(index, &solana_sdk::loader_v4::id(), &feature_set),
            ProgramKind::NotBuiltin
        );

        // builtin return default cost
        index += 1;
        assert_eq!(
            test_store.get_program_kind(index, &solana_sdk::loader_v4::id(), &feature_set),
            ProgramKind::Builtin {
                is_compute_budget: false
            }
        );

        // compute-budget return default cost, and true flag
        index += 1;
        assert_eq!(
            test_store.get_program_kind(index, &solana_sdk::compute_budget::id(), &feature_set),
            ProgramKind::Builtin {
                is_compute_budget: true
            }
        );
    }

    #[test]
    fn test_test_get_program_kind() {
        get_program_kind(&FeatureSet::default());
        get_program_kind(&FeatureSet::all_enabled());
    }

    #[test]
    #[should_panic(expected = "program id index is sanitized")]
    fn test_get_program_kind_out_of_bound_index() {
        let mut test_store = ComputeBudgetProgramIdFilter::new();
        assert_eq!(
            test_store
                .get_program_kind(FILTER_SIZE as usize + 1, &DUMMY_PROGRAM_ID.parse().unwrap(), &FeatureSet::default()),
            ProgramKind::NotBuiltin
        );

        assert_eq!(
            test_store
                .get_program_kind(FILTER_SIZE as usize + 1, &DUMMY_PROGRAM_ID.parse().unwrap(), &FeatureSet::all_enabled()),
            ProgramKind::NotBuiltin
        );
    }
}
