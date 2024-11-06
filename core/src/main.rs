use borsh::BorshDeserialize;
use circuits::{
    header_chain::{
        BlockHeader, BlockHeaderCircuitOutput, HeaderChainCircuitInput, HeaderChainPrevProofType,
    },
    risc0_zkvm::{default_prover, ExecutorEnv},
};

use header_chain_circuit::{HEADER_CHAIN_GUEST_ELF, HEADER_CHAIN_GUEST_ID};
use risc0_zkvm::{ProverOpts, Receipt, SuccinctReceiptVerifierParameters};
use sha2::Digest;
use sha2::Sha256;
use std::{env, fs};

pub mod docker;

fn main() {
    // Parse command-line arguments
    let args: Vec<String> = env::args().collect();
    if args.len() < 4 {
        eprintln!("Usage: <program> <input_proof> <output_file_path> <batch_size>");
        return;
    }

    let input_proof = &args[1];
    let output_file_path = &args[2];
    let batch_size: usize = args[3].parse().expect("Batch size should be a number");

    // Download the headers.bin file from https://zerosync.org/chaindata/headers.bin
    let headers = include_bytes!("../../headers.bin");
    let headers = headers
        .chunks(80)
        .map(|header| BlockHeader::try_from_slice(header).unwrap())
        .collect::<Vec<BlockHeader>>();

    // Set the previous proof type based on input_proof argument
    let prev_receipt = if input_proof.to_lowercase() == "none" {
        None
    } else {
        let proof_bytes = fs::read(input_proof).expect("Failed to read input proof file");
        let receipt: Receipt = Receipt::try_from_slice(&proof_bytes).unwrap();
        Some(receipt)
    };

    let mut start = 0;
    let prev_proof = match prev_receipt.clone() {
        Some(receipt) => {
            let output =
                BlockHeaderCircuitOutput::try_from_slice(&receipt.journal.bytes.clone()).unwrap();
            start = output.chain_state.block_height as usize + 1;
            HeaderChainPrevProofType::PrevProof(output)
        }
        None => HeaderChainPrevProofType::GenesisBlock,
    };

    // Prepare the input for the circuit
    let input = HeaderChainCircuitInput {
        method_id: HEADER_CHAIN_GUEST_ID,
        prev_proof,
        block_headers: headers[start..start + batch_size].to_vec(),
    };

    // Build ENV
    let mut binding = ExecutorEnv::builder();
    let mut env = binding.write_slice(&borsh::to_vec(&input).unwrap());
    if let Some(receipt) = prev_receipt {
        env = env.add_assumption(receipt);
    }
    let env = env.build().unwrap();

    // Obtain the default prover.
    let prover = default_prover();

    // Produce a receipt by proving the specified ELF binary.
    let receipt = prover
        .prove_with_opts(env, HEADER_CHAIN_GUEST_ELF, &ProverOpts::succinct())
        .unwrap()
        .receipt;

    // Extract journal of receipt
    let output = BlockHeaderCircuitOutput::try_from_slice(&receipt.journal.bytes).unwrap();

    println!("Output: {:#?}", output.method_id);

    // Save the receipt to the specified output file path
    let receipt_bytes = borsh::to_vec(&receipt).unwrap();
    fs::write(output_file_path, &receipt_bytes).expect("Failed to write receipt to output file");
    println!("Receipt saved to {}", output_file_path);
}

/// control_root, pre_state_digest, post_state_digest, id_bn254_fr
pub fn calculate_succinct_output_prefix(method_id: &[u8]) -> [u8; 32] {
    let succinct_verifier_params = SuccinctReceiptVerifierParameters::default();
    println!("Succinct verifier params: {:?}", succinct_verifier_params);
    let succinct_control_root = succinct_verifier_params.control_root;
    println!("Succinct control root: {:?}", succinct_control_root);
    let mut succinct_control_root_bytes: [u8; 32] =
        succinct_control_root.as_bytes().try_into().unwrap();
    // succinct_control_root_bytes.reverse();
    for byte in succinct_control_root_bytes.iter_mut() {
        *byte = byte.reverse_bits();
    }

    // let mut pre_state_bits: Vec<u8> = Vec::new();
    // for item in method_id.iter().take(8) {
    //     for j in 0..4 {
    //         for k in 0..8 {
    //             pre_state_bits.push((item >> (8 * j + 7 - k)) as u8 & 1);
    //         }
    //     }
    // };
    let pre_state_bytes = method_id.to_vec();
    println!("pre_state_bytes: {:?}", pre_state_bytes);

    let control_id_bytes =
        hex::decode("4e160df1e119ac0e3d658755a9edf38c8feb307b34bc10b57f4538dbe122a005").unwrap(); // id_bn254_fr
                                                                                                  // let ident_receipt = risc0_zkvm::recursion::identity_p254(SuccinctReceipt<ReceiptClaim>::);

    let post_state_bytes =
        hex::decode("a3acc27117418996340b84e5a90f3ef4c49d22c79e44aad822ec9c313e1eb8e2").unwrap(); // post_state_digest

    let mut hasher = Sha256::new();
    hasher.update(&succinct_control_root_bytes);
    hasher.update(&pre_state_bytes);
    hasher.update(&post_state_bytes);
    hasher.update(&control_id_bytes);
    let result: [u8; 32] = hasher
        .finalize()
        .try_into()
        .expect("SHA256 should produce a 32-byte output");

    result
}

#[cfg(test)]
mod tests {
    use docker::stark_to_succinct;
    use risc0_zkvm::compute_image_id;

    use super::*;
    // #[ignore = "This is to only test final proof generation"]
    #[test]
    fn test_final_circuit() {
        let final_circuit_elf = include_bytes!(
            "../../target/riscv-guest/riscv32im-risc0-zkvm-elf/docker/final_guest/final-guest"
        );
        let final_proof = include_bytes!("../../first_10.bin");
        let final_circuit_id = compute_image_id(final_circuit_elf).unwrap();
        println!("final circuit id: {:#?}", final_circuit_id);

        println!(
            "final circuit id: {}",
            compute_image_id(final_circuit_elf).unwrap()
        );

        let receipt: Receipt = Receipt::try_from_slice(final_proof).unwrap();

        let output = BlockHeaderCircuitOutput::try_from_slice(&receipt.journal.bytes).unwrap();

        let env = ExecutorEnv::builder()
            .write_slice(&borsh::to_vec(&output).unwrap())
            .add_assumption(receipt)
            .build()
            .unwrap();

        let prover = default_prover();

        let receipt = prover
            .prove_with_opts(env, final_circuit_elf, &ProverOpts::succinct())
            .unwrap()
            .receipt;

        let succinct_receipt = receipt.inner.succinct().unwrap().clone();
        let receipt_claim = succinct_receipt.clone().claim;
        println!("Receipt claim: {:#?}", receipt_claim);
        let journal: [u8; 32] = receipt.journal.bytes.clone().try_into().unwrap();
        let (proof, output_json_bytes) =
            stark_to_succinct(succinct_receipt, &receipt.journal.bytes);
        print!("Proof: {:#?}", proof);
        let constants_digest = calculate_succinct_output_prefix(final_circuit_id.as_bytes());
        println!("Constants digest: {:#?}", constants_digest);
        println!("Journal: {:#?}", receipt.journal);
        let mut constants_blake3_input: [u8; 32] = [0; 32];
        for i in 0..8 {
            let mut temp: u32 =
                u32::from_be_bytes(constants_digest[4 * i..4 * i + 4].try_into().unwrap());
            temp = temp.reverse_bits();
            constants_blake3_input[4 * i..4 * i + 4].copy_from_slice(&temp.to_le_bytes());
        }
        let mut journal_blake3_input: [u8; 32] = [0; 32];
        for i in 0..8 {
            let mut temp: u32 = u32::from_be_bytes(journal[4 * i..4 * i + 4].try_into().unwrap());
            temp = temp.reverse_bits();
            journal_blake3_input[4 * i..4 * i + 4].copy_from_slice(&temp.to_le_bytes());
        }
        println!("Constants blake3 input: {:#?}", constants_blake3_input);
        println!("Journal blake3 input: {:#?}", journal_blake3_input);
        let mut hasher = blake3::Hasher::new();
        hasher.update(&constants_blake3_input);
        hasher.update(&journal_blake3_input);
        let final_output = hasher.finalize();
        println!("Final output: {:#?}", final_output);
        let final_output_bytes: [u8; 32] = final_output.try_into().unwrap();
        let final_output_trimmed: [u8; 31] = final_output_bytes[..31].try_into().unwrap();
        assert_eq!(final_output_trimmed, output_json_bytes);
    }
}
