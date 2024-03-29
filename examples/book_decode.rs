use std::fs::File;

use ola_lang_abi::Abi;

fn main() {
    // Parse ABI JSON file
    let abi: Abi = {
        let file = File::open("examples/BookExample.json").expect("failed to open ABI file");

        serde_json::from_reader(file).expect("failed to parse ABI")
    };

    let input_data = vec![60, 5, 111, 108, 97, 118, 109, 7, 120553111];

    // Decode input
    let (func, decoded_data) = abi.decode_input_from_slice(&input_data).unwrap();

    println!(
        "decode function input {:?}\n data {:?}",
        func.name, decoded_data
    );

    // Decode output "hello"
    let output_data = vec![5, 104, 101, 108, 108, 111, 6];

    let decoded_data = abi
        .decode_output_from_slice(abi.functions[1].signature().as_str(), &output_data)
        .unwrap();

    println!("decode function output {:?}\n", decoded_data);
}
