----
Priority Fee
----

## Scratch 
- is it prioryt of block inclusion, or priority of state access
- is it guaraneteed? 
- is it enforced by protocol?
- is it necessary to add priority fee per account?
- should Priority fee count towards vote credits and staking rewards?

### Case of "Priotity Fee is a optional fee paid to Block Producer to prioritize transaction's block inclusion"
- Producer can include lower priority fee tx to block over higher one (eg not enfored) for various reasons:
  - side deals, A: 100% reward to discourage;
  - determines a higher prio fee tx may lower entire block profit; A: this works in free market princepal, submitter just need to raise prio fee.

## Terminology

Solana Doc term def: https://docs.solana.com/terminology#prioritization-fee

### Priority Fee
An optional fee that a Transaction Submitter willing to pay to prioritize its transaction's block inclusion; it
- is denominated in "Milli-Lamport/Compute-Unit"
- is a bid for Early Inclusion: The Priority Fee serves as a bid to prioritize the inclusion of the associated transaction in a block, it does not provide a guarantee. Inclusion ultimately depends on the discretion of the Block Producer, considering profitability.
- is 100% rewarded to Block Producer: If the transaction associated with the Priority Fee successfully lands in a block, a deterministic `Lamport`s , calculated from `Priotity Fee * Resources cost`, is rewarded to the Block Producer;

In summary, the Priority Fee is a mechanism allowing Transaction Submitters to potentially expedite the inclusion of their transactions in a block by offering an optional fee, denominated in `Milli-Lamports/Compute-units`, which is rewarded to the Block Producer in the event of successful transaction inclusion.








- in Lab client, priority for inclusion implies priority for state access (txs in first entry can access accounts before txs in second entry, but this is just how Lab Client is implemented, not enforced by protocol. Some optimization on Replay may break it - hence need to define Priority fee) []: should the def to include priority of state access?


### Local Fee Market
- what it is / what it meant to
- market data emitting via RPC


