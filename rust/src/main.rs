#![allow(unused)]
use bitcoin::hex::DisplayHex;
use bitcoincore_rpc::bitcoin::Amount;
use bitcoincore_rpc::{Auth, Client, RpcApi};
use serde::Deserialize;
use serde_json::json;
use std::fs::File;
use std::io::Write;

// Node access params
const RPC_URL: &str = "http://127.0.0.1:18443"; // Default regtest RPC port
const RPC_USER: &str = "alice";
const RPC_PASS: &str = "password";

// You can use calls not provided in RPC lib API using the generic `call` function.
// An example of using the `send` RPC call, which doesn't have exposed API.
// You can also use serde_json `Deserialize` derivation to capture the returned json result.
fn send(rpc: &Client, addr: &str) -> bitcoincore_rpc::Result<String> {
    let args = [
        json!([{addr : 100 }]), // recipient address
        json!(null),            // conf target
        json!(null),            // estimate mode
        json!(null),            // fee rate in sats/vb
        json!(null),            // Empty option object
    ];

    #[derive(Deserialize)]
    struct SendResult {
        complete: bool,
        txid: String,
    }
    let send_result = rpc.call::<SendResult>("send", &args)?;
    assert!(send_result.complete);
    Ok(send_result.txid)
}

fn main() -> bitcoincore_rpc::Result<()> {
    // Connect to Bitcoin Core RPC using credentials
    let rpc = Client::new(
        RPC_URL,
        Auth::UserPass(RPC_USER.to_owned(), RPC_PASS.to_owned()),
    )?;

    // Get blockchain info to verify we're connected
    let blockchain_info = rpc.get_blockchain_info()?;
    println!("Blockchain Info: {:?}", blockchain_info);

    // -------------------------------------------------------------------------
    // Create/Load wallets named 'Miner' and 'Trader'
    // We attempt to create each wallet; if it already exists we load it instead.
    // Bitcoin Core requires wallets to be explicitly created or loaded before use.
    // -------------------------------------------------------------------------
    for wallet_name in &["Miner", "Trader"] {
        let create_result = rpc.create_wallet(wallet_name, None, None, None, None);
        match create_result {
            Ok(_) => println!("Created wallet: {}", wallet_name),
            Err(e) => {
                // Wallet may already exist — try loading it
                let load_result = rpc.load_wallet(wallet_name);
                match load_result {
                    Ok(_) => println!("Loaded wallet: {}", wallet_name),
                    Err(load_err) => {
                        // Already loaded is fine — ignore that specific error
                        println!("Wallet '{}' already loaded or error: {:?}", wallet_name, load_err);
                    }
                }
            }
        }
    }

    // Connect to the Miner wallet endpoint for wallet-specific RPC calls
    let miner_rpc = Client::new(
        &format!("{}/wallet/Miner", RPC_URL),
        Auth::UserPass(RPC_USER.to_owned(), RPC_PASS.to_owned()),
    )?;

    // Connect to the Trader wallet endpoint
    let trader_rpc = Client::new(
        &format!("{}/wallet/Trader", RPC_URL),
        Auth::UserPass(RPC_USER.to_owned(), RPC_PASS.to_owned()),
    )?;

    // -------------------------------------------------------------------------
    // Generate a new address in the Miner wallet with label "Mining Reward"
    // This address will receive all block subsidy rewards.
    // -------------------------------------------------------------------------
    let miner_address = miner_rpc.get_new_address(Some("Mining Reward"), None)?;
    let miner_address = miner_address.require_network(bitcoincore_rpc::bitcoin::Network::Regtest)
        .expect("Miner address must be regtest");
    println!("Miner address: {}", miner_address);

    // -------------------------------------------------------------------------
    // Mine blocks until Miner has a positive spendable balance.
    //
    // WHY 101 BLOCKS?
    // In Bitcoin, coinbase (block reward) outputs are subject to a maturity rule:
    // they cannot be spent until 100 more blocks have been mined on top of them.
    // This means we need to mine at least 101 blocks so that the first block's
    // reward becomes spendable (100 confirmations on top of block 1).
    // -------------------------------------------------------------------------
    let blocks = miner_rpc.generate_to_address(101, &miner_address)?;
    println!("Mined {} blocks", blocks.len());

    // Print the Miner wallet balance
    let miner_balance = miner_rpc.get_balance(None, None)?;
    println!("Miner wallet balance: {} BTC", miner_balance.to_btc());

    // -------------------------------------------------------------------------
    // Create a receiving address in the Trader wallet labeled "Received"
    // -------------------------------------------------------------------------
    let trader_address = trader_rpc.get_new_address(Some("Received"), None)?;
    let trader_address = trader_address.require_network(bitcoincore_rpc::bitcoin::Network::Regtest)
        .expect("Trader address must be regtest");
    println!("Trader address: {}", trader_address);

    // -------------------------------------------------------------------------
    // Send 20 BTC from Miner to Trader
    // -------------------------------------------------------------------------
    let send_amount = Amount::from_btc(20.0).unwrap();
    let txid = miner_rpc.send_to_address(
        &trader_address,
        send_amount,
        None, // comment
        None, // comment_to
        None, // subtract_fee_from_amount
        None, // replaceable
        None, // conf_target
        None, // estimate_mode
    )?;
    println!("Transaction sent: {}", txid);

    // -------------------------------------------------------------------------
    // Fetch the unconfirmed transaction from the mempool
    // -------------------------------------------------------------------------
    let mempool_entry = rpc.get_mempool_entry(&txid)?;
    println!("Mempool entry: {:?}", mempool_entry);

    // -------------------------------------------------------------------------
    // Mine 1 block to confirm the transaction
    // -------------------------------------------------------------------------
    let confirm_blocks = miner_rpc.generate_to_address(1, &miner_address)?;
    println!("Mined 1 confirmation block");

    // -------------------------------------------------------------------------
    // Extract all required transaction details
    // -------------------------------------------------------------------------

    // Get the raw transaction with verbose output
    let raw_tx = rpc.get_raw_transaction_info(&txid, None)?;

    // Get the block in which this transaction was confirmed
    let block_hash = raw_tx.blockhash.expect("Transaction should be confirmed");
    let block_info = rpc.get_block_info(&block_hash)?;
    let block_height = block_info.height;

    // Identify Trader's output (20 BTC) and Miner's change output
    let mut trader_output_address = String::new();
    let mut trader_output_amount = 0.0f64;
    let mut miner_change_address = String::new();
    let mut miner_change_amount = 0.0f64;

    for vout in &raw_tx.vout {
        let value = vout.value.to_btc();
        // The output closest to 20 BTC is the payment to Trader
        if (value - 20.0).abs() < 0.01 {
            trader_output_amount = value;
            if let Some(addr) = vout.script_pub_key.address.as_ref() {
                trader_output_address = addr.clone()
                    .require_network(bitcoincore_rpc::bitcoin::Network::Regtest)
                    .map(|a| a.to_string())
                    .unwrap_or_default();
            }
        } else {
            // The other output is Miner's change
            miner_change_amount = value;
            if let Some(addr) = vout.script_pub_key.address.as_ref() {
                miner_change_address = addr.clone()
                    .require_network(bitcoincore_rpc::bitcoin::Network::Regtest)
                    .map(|a| a.to_string())
                    .unwrap_or_default();
            }
        }
    }

    // Resolve all inputs: sum up all input amounts and use the first input's address
    // as the "Miner's Input Address". Bitcoin transactions can have multiple inputs
    // (UTXOs) to cover the desired spend amount.
    let mut miner_input_amount = 0.0f64;
    let mut miner_input_address = String::new();

    for (i, input) in raw_tx.vin.iter().enumerate() {
        let prev_txid = input.txid.expect("Input must have a txid");
        let prev_vout_index = input.vout.expect("Input must have a vout index") as usize;

        let prev_tx = rpc.get_raw_transaction_info(&prev_txid, None)?;
        let prev_output = &prev_tx.vout[prev_vout_index];
        miner_input_amount += prev_output.value.to_btc();

        // Use the first input's address as the representative Miner input address
        if i == 0 {
            miner_input_address = prev_output
                .script_pub_key
                .address
                .as_ref()
                .map(|a| {
                    a.clone()
                        .require_network(bitcoincore_rpc::bitcoin::Network::Regtest)
                        .map(|a| a.to_string())
                        .unwrap_or_default()
                })
                .unwrap_or_default();
        }
    }

    // Calculate transaction fee = total inputs - total outputs
    let total_output = trader_output_amount + miner_change_amount;
    let tx_fee = miner_input_amount - total_output;

    println!("--- Transaction Details ---");
    println!("TXID:                  {}", txid);
    println!("Miner Input Address:   {}", miner_input_address);
    println!("Miner Input Amount:    {} BTC", miner_input_amount);
    println!("Trader Output Address: {}", trader_output_address);
    println!("Trader Output Amount:  {} BTC", trader_output_amount);
    println!("Miner Change Address:  {}", miner_change_address);
    println!("Miner Change Amount:   {} BTC", miner_change_amount);
    println!("Transaction Fee:       {} BTC", tx_fee);
    println!("Block Height:          {}", block_height);
    println!("Block Hash:            {}", block_hash);

    // -------------------------------------------------------------------------
    // Write the output to ../out.txt in the required format
    // -------------------------------------------------------------------------
    let out_path = "../out.txt";
    let mut file = File::create(out_path)?;
    writeln!(file, "{}", txid)?;
    writeln!(file, "{}", miner_input_address)?;
    writeln!(file, "{}", miner_input_amount)?;
    writeln!(file, "{}", trader_output_address)?;
    writeln!(file, "{}", trader_output_amount)?;
    writeln!(file, "{}", miner_change_address)?;
    writeln!(file, "{}", miner_change_amount)?;
    writeln!(file, "{}", -tx_fee)?;
    writeln!(file, "{}", block_height)?;
    writeln!(file, "{}", block_hash)?;

    println!("Output written to {}", out_path);

    Ok(())
}
