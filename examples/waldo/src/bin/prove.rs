// Copyright 2023 RISC Zero, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::{error::Error, fs, path::PathBuf};

use clap::Parser;
use image::{io::Reader as ImageReader, GenericImageView};
use risc0_zkvm::{default_executor_from_elf, serde, ExecutorEnv};
use waldo_core::{
    image::{ImageMask, ImageMerkleTree, IMAGE_CHUNK_SIZE},
    merkle::SYS_VECTOR_ORACLE,
    PrivateInput,
};
use waldo_methods::IMAGE_CROP_ELF;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    /// Input file path to the full Where's Waldo image.
    #[clap(short = 'i', long, value_parser, value_hint = clap::ValueHint::FilePath)]
    image: PathBuf,

    /// X coordinate, in pixels from the top-left corner, of Waldo.
    #[clap(short = 'x', long, value_parser)]
    waldo_x: u32,

    /// Y coordinate, in pixels from the top-left corner, of Waldo.
    #[clap(short = 'y', long, value_parser)]
    waldo_y: u32,

    /// Width, in pixels, of the cutout for Waldo.
    #[clap(long, value_parser)]
    width: u32,

    /// Height, in pixels, of the cutout for Waldo.
    #[clap(long, value_parser)]
    height: u32,

    /// Optional input file path to an image mask to apply to Waldo.
    /// Grayscale pixel values will be subtracted from the cropped image of
    /// Waldo such that a black pixel in the mask will result in the
    /// corresponding image pixel being blacked out. Must be the same
    /// dimensions, in pixels, as the cut out x and y.
    #[clap(short = 'm', long, value_parser, value_hint = clap::ValueHint::FilePath)]
    mask: Option<PathBuf>,

    /// Output file path to save the receipt. Note that the receipt contains the
    /// cutout of waldo.
    #[clap(short = 'r', long, value_parser, default_value = "./receipt.bin", value_hint = clap::ValueHint::FilePath)]
    receipt: PathBuf,
}

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init();

    let args = Args::parse();

    // Read the image from disk.
    let img = ImageReader::open(&args.image)?.decode()?;
    println!(
        "Read image at {} with size: {} x {}",
        &args.image.display(),
        img.width(),
        img.height()
    );

    let crop_location = (args.waldo_x, args.waldo_y);
    let crop_dimensions = (args.width, args.height);

    // Read the image mask from disk, if provided.
    let mask = args.mask.map_or(Ok::<_, Box<dyn Error>>(None), |path| {
        // Read the image mask from disk. Reads any format and color image.
        let mask: ImageMask = ImageReader::open(&path)?.decode()?.into();
        if mask.dimensions() != crop_dimensions {
            return Err(format!(
                "Mask dimensions do not match specified height and width for Waldo: {:?} != {:?}",
                mask.dimensions(),
                crop_dimensions
            )
            .into());
        }
        println!("Read image mask at {}", &path.display(),);

        Ok(Some(mask.into_raw()))
    })?;

    // Construct a Merkle tree from the full Where's Waldo image.
    let img_merkle_tree = ImageMerkleTree::<{ IMAGE_CHUNK_SIZE }>::new(&img);
    println!(
        "Created Merkle tree from image with root {:?}",
        img_merkle_tree.root(),
    );

    // Give the private input to the guest, including Waldo's location.
    let input = PrivateInput {
        root: img_merkle_tree.root(),
        image_dimensions: img.dimensions(),
        mask,
        crop_location,
        crop_dimensions,
    };

    // Make the ExecutorEnv, registering an io_callback to communicate
    // vector oracle data from the Merkle tree.
    let env = ExecutorEnv::builder()
        .add_input(&serde::to_vec(&input)?)
        .io_callback(SYS_VECTOR_ORACLE, img_merkle_tree.vector_oracle_callback())
        .build()
        .unwrap();

    // Run prover and generate receipt
    println!(
        "Running the prover to cut out Waldo at {:?} with dimensions {:?}",
        input.crop_location, input.crop_dimensions,
    );
    // Make the Executor, loading the image crop method binary.
    let mut exec = default_executor_from_elf(env, IMAGE_CROP_ELF)?;
    let session = exec.run()?;
    let receipt = session.prove()?;

    // Save the receipt to disk so it can be sent to the verifier.
    fs::write(&args.receipt, bincode::serialize(&receipt).unwrap())?;
    println!("Success! Saved the receipt to {}", &args.receipt.display());

    Ok(())
}
