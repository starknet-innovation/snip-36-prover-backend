#[starknet::interface]
trait ICounter<TContractState> {
    fn increment(ref self: TContractState, amount: felt252);
    fn get_counter(self: @TContractState) -> felt252;
}

#[starknet::contract]
mod Counter {
    use starknet::storage::{StoragePointerReadAccess, StoragePointerWriteAccess};

    #[storage]
    struct Storage {
        counter: felt252,
    }

    #[abi(embed_v0)]
    impl CounterImpl of super::ICounter<ContractState> {
        fn increment(ref self: ContractState, amount: felt252) {
            self.counter.write(self.counter.read() + amount);
        }

        fn get_counter(self: @ContractState) -> felt252 {
            self.counter.read()
        }
    }
}

#[starknet::interface]
trait IMessenger<TContractState> {
    fn send_message(ref self: TContractState, to_address: felt252, payload: Span<felt252>);
}

#[starknet::contract]
mod Messenger {
    use starknet::syscalls::send_message_to_l1_syscall;

    #[storage]
    struct Storage {}

    #[abi(embed_v0)]
    impl MessengerImpl of super::IMessenger<ContractState> {
        fn send_message(ref self: ContractState, to_address: felt252, payload: Span<felt252>) {
            send_message_to_l1_syscall(to_address, payload).unwrap();
        }
    }
}

// ── ERC20 interface for dispatcher generation ───────────────────────────────
#[starknet::interface]
trait IERC20<TContractState> {
    fn transfer_from(
        ref self: TContractState,
        sender: starknet::ContractAddress,
        recipient: starknet::ContractAddress,
        amount: u256,
    ) -> bool;
    fn transfer(
        ref self: TContractState,
        recipient: starknet::ContractAddress,
        amount: u256,
    ) -> bool;
    fn balance_of(self: @TContractState, account: starknet::ContractAddress) -> u256;
}

// ── CoinFlipBank: on-chain settlement with real STRK deposits ───────────────

#[starknet::interface]
trait ICoinFlipBank<TContractState> {
    /// Player calls after ERC20 approve: deposits stake for a game session.
    fn deposit(
        ref self: TContractState,
        session_id: felt252,
        bet_amount: u256,
        seed: felt252,
        bet: felt252,
    );
    /// Owner (server) calls: matches the player's deposit with bank funds.
    fn match_deposit(ref self: TContractState, session_id: felt252);
    /// Owner calls after SNIP-36 proof: settles based on deterministic outcome.
    fn settle(ref self: TContractState, session_id: felt252, seed: felt252);
    /// Player calls to withdraw accumulated winnings.
    fn withdraw(ref self: TContractState);
    /// View: get game state (player_felt, bet_amount, seed, bet, state).
    fn get_game(self: @TContractState, session_id: felt252) -> (felt252, u256, felt252, felt252, u8);
    /// View: get player's withdrawable balance.
    fn get_balance(self: @TContractState, player: starknet::ContractAddress) -> u256;
}

#[starknet::contract]
mod CoinFlipBank {
    use starknet::storage::{
        StoragePointerReadAccess, StoragePointerWriteAccess,
        StorageMapReadAccess, StorageMapWriteAccess, Map,
    };
    use starknet::{ContractAddress, get_caller_address, get_contract_address};
    use core::pedersen::pedersen;
    use super::{IERC20Dispatcher, IERC20DispatcherTrait};

    /// STRK token address on Sepolia.
    const STRK_TOKEN: felt252 =
        0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d;

    /// Game states: 0=empty, 1=player deposited, 2=bank matched, 3=settled.
    const STATE_EMPTY: u8 = 0;
    const STATE_DEPOSITED: u8 = 1;
    const STATE_MATCHED: u8 = 2;
    const STATE_SETTLED: u8 = 3;

    #[storage]
    struct Storage {
        owner: ContractAddress,
        // session_id -> game fields (stored individually since Map<K, struct> needs Store derive)
        game_player: Map<felt252, ContractAddress>,
        game_amount: Map<felt252, u256>,
        game_seed: Map<felt252, felt252>,
        game_bet: Map<felt252, felt252>,
        game_state: Map<felt252, u8>,
        // Withdrawable balances per player
        player_balance: Map<ContractAddress, u256>,
    }

    #[constructor]
    fn constructor(ref self: ContractState, owner: ContractAddress) {
        self.owner.write(owner);
    }

    #[abi(embed_v0)]
    impl CoinFlipBankImpl of super::ICoinFlipBank<ContractState> {
        fn deposit(
            ref self: ContractState,
            session_id: felt252,
            bet_amount: u256,
            seed: felt252,
            bet: felt252,
        ) {
            let caller = get_caller_address();
            assert(bet_amount > 0, 'Amount must be > 0');
            assert(self.game_state.read(session_id) == STATE_EMPTY, 'Session already exists');

            // Transfer STRK from player to this contract
            let strk = IERC20Dispatcher {
                contract_address: STRK_TOKEN.try_into().unwrap(),
            };
            strk.transfer_from(caller, get_contract_address(), bet_amount);

            // Record game
            self.game_player.write(session_id, caller);
            self.game_amount.write(session_id, bet_amount);
            self.game_seed.write(session_id, seed);
            self.game_bet.write(session_id, bet);
            self.game_state.write(session_id, STATE_DEPOSITED);
        }

        fn match_deposit(ref self: ContractState, session_id: felt252) {
            let caller = get_caller_address();
            assert(caller == self.owner.read(), 'Only owner');
            assert(self.game_state.read(session_id) == STATE_DEPOSITED, 'Not deposited');

            let amount = self.game_amount.read(session_id);

            // Transfer matching STRK from owner to this contract
            let strk = IERC20Dispatcher {
                contract_address: STRK_TOKEN.try_into().unwrap(),
            };
            strk.transfer_from(caller, get_contract_address(), amount);

            self.game_state.write(session_id, STATE_MATCHED);
        }

        fn settle(ref self: ContractState, session_id: felt252, seed: felt252) {
            let caller = get_caller_address();
            assert(caller == self.owner.read(), 'Only owner');
            assert(self.game_state.read(session_id) == STATE_MATCHED, 'Not matched');

            let player = self.game_player.read(session_id);
            let amount = self.game_amount.read(session_id);
            let bet = self.game_bet.read(session_id);

            // Deterministic outcome — same logic as CoinFlip.play()
            let player_felt: felt252 = player.into();
            let hash = pedersen(seed, player_felt);
            let hash_u256: u256 = hash.into();
            let outcome: felt252 = if hash_u256.low % 2 == 0 {
                0
            } else {
                1
            };

            let payout = amount * 2;
            let strk = IERC20Dispatcher {
                contract_address: STRK_TOKEN.try_into().unwrap(),
            };

            if outcome == bet {
                // Player wins: transfer 2x directly to player wallet
                strk.transfer(player, payout);
            } else {
                // Bank wins: transfer 2x back to owner
                strk.transfer(self.owner.read(), payout);
            }

            self.game_state.write(session_id, STATE_SETTLED);
        }

        fn withdraw(ref self: ContractState) {
            let caller = get_caller_address();
            let balance = self.player_balance.read(caller);
            assert(balance > 0, 'Nothing to withdraw');

            self.player_balance.write(caller, 0);

            let strk = IERC20Dispatcher {
                contract_address: STRK_TOKEN.try_into().unwrap(),
            };
            strk.transfer(caller, balance);
        }

        fn get_game(
            self: @ContractState, session_id: felt252,
        ) -> (felt252, u256, felt252, felt252, u8) {
            let player: felt252 = self.game_player.read(session_id).into();
            (
                player,
                self.game_amount.read(session_id),
                self.game_seed.read(session_id),
                self.game_bet.read(session_id),
                self.game_state.read(session_id),
            )
        }

        fn get_balance(self: @ContractState, player: ContractAddress) -> u256 {
            self.player_balance.read(player)
        }
    }
}

/// Provable coin flip: deterministic outcome from public inputs, settled via L2→L1 message.
///
/// Demonstrates using SNIP-36 virtual blocks as a verifiable computation oracle:
/// - Public inputs (seed + player address) go in via calldata
/// - Deterministic PRNG (Poseidon hash) computes the outcome
/// - Settlement receipt is emitted as an L2→L1 message
/// - The stwo proof guarantees the computation was honest
#[starknet::interface]
trait ICoinFlip<TContractState> {
    fn play(ref self: TContractState, seed: felt252, player: felt252, bet: felt252);
}

#[starknet::contract]
mod CoinFlip {
    use starknet::syscalls::send_message_to_l1_syscall;
    use core::pedersen::pedersen;

    /// L1 settlement contract address (placeholder).
    const SETTLEMENT_ADDRESS: felt252 = 0x1;

    #[storage]
    struct Storage {}

    #[abi(embed_v0)]
    impl CoinFlipImpl of super::ICoinFlip<ContractState> {
        fn play(ref self: ContractState, seed: felt252, player: felt252, bet: felt252) {
            // Deterministic outcome from public inputs
            let hash = pedersen(seed, player);
            let hash_u256: u256 = hash.into();
            let outcome: felt252 = if hash_u256.low % 2 == 0 { 0 } else { 1 };

            // 1 if player guessed correctly, 0 otherwise
            let won: felt252 = if outcome == bet { 1 } else { 0 };

            // Emit settlement receipt as L2→L1 message
            // Payload: [player, seed, bet, outcome, won]
            send_message_to_l1_syscall(
                SETTLEMENT_ADDRESS,
                array![player, seed, bet, outcome, won].span(),
            ).unwrap();
        }
    }
}
