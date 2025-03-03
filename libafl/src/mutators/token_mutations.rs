//! Tokens are what afl calls extras or dictionaries.
//! They may be inserted as part of mutations during fuzzing.
use alloc::vec::Vec;
use core::{marker::PhantomData, mem::size_of};
use serde::{Deserialize, Serialize};

#[cfg(feature = "std")]
use crate::mutators::str_decode;
#[cfg(feature = "std")]
use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
};

use crate::{
    bolts::rands::Rand,
    inputs::{HasBytesVec, Input},
    mutators::{buffer_self_copy, mutations::buffer_copy, MutationResult, Mutator, Named},
    observers::cmp::{CmpValues, CmpValuesMetadata},
    state::{HasMaxSize, HasMetadata, HasRand},
    Error,
};

/// A state metadata holding a list of tokens
#[derive(Debug, Serialize, Deserialize)]
pub struct Tokens {
    token_vec: Vec<Vec<u8>>,
}

crate::impl_serdeany!(Tokens);

/// The metadata used for token mutators
impl Tokens {
    /// Creates a new tokens metadata (old-skool afl name: `dictornary`)
    #[must_use]
    pub fn new(token_vec: Vec<Vec<u8>>) -> Self {
        Self { token_vec }
    }

    /// Creates a new instance from a file
    #[cfg(feature = "std")]
    pub fn from_tokens_file<P>(file: P) -> Result<Self, Error>
    where
        P: AsRef<Path>,
    {
        let mut ret = Self::new(vec![]);
        ret.add_tokens_from_file(file)?;
        Ok(ret)
    }

    /// Adds a token to a dictionary, checking it is not a duplicate
    /// Returns `false` if the token was already present and did not get added.
    #[allow(clippy::ptr_arg)]
    pub fn add_token(&mut self, token: &Vec<u8>) -> bool {
        if self.token_vec.contains(token) {
            return false;
        }
        self.token_vec.push(token.clone());
        true
    }

    /// Reads a tokens file, returning the count of new entries read
    #[cfg(feature = "std")]
    pub fn add_tokens_from_file<P>(&mut self, file: P) -> Result<u32, Error>
    where
        P: AsRef<Path>,
    {
        let mut entries = 0;

        // println!("Loading tokens file {:?} ...", file);

        let file = File::open(file)?; // panic if not found
        let reader = BufReader::new(file);

        for line in reader.lines() {
            let line = line.unwrap();
            let line = line.trim_start().trim_end();

            // we are only interested in '"..."', not prefixed 'foo = '
            let start = line.chars().next();
            if line.is_empty() || start == Some('#') {
                continue;
            }
            let pos_quote = match line.find('\"') {
                Some(x) => x,
                None => return Err(Error::IllegalArgument("Illegal line: ".to_owned() + line)),
            };
            if line.chars().nth(line.len() - 1) != Some('"') {
                return Err(Error::IllegalArgument("Illegal line: ".to_owned() + line));
            }

            // extract item
            let item = match line.get(pos_quote + 1..line.len() - 1) {
                Some(x) => x,
                None => return Err(Error::IllegalArgument("Illegal line: ".to_owned() + line)),
            };
            if item.is_empty() {
                continue;
            }

            // decode
            let token: Vec<u8> = match str_decode(item) {
                Ok(val) => val,
                Err(_) => {
                    return Err(Error::IllegalArgument(
                        "Illegal line (hex decoding): ".to_owned() + line,
                    ))
                }
            };

            // add
            if self.add_token(&token) {
                entries += 1;
            }
        }

        Ok(entries)
    }

    /// Gets the tokens stored in this db
    #[must_use]
    pub fn tokens(&self) -> &[Vec<u8>] {
        &self.token_vec
    }
}

/// Inserts a random token at a random position in the `Input`.
#[derive(Debug, Default)]
pub struct TokenInsert<I, R, S>
where
    I: Input + HasBytesVec,
    S: HasMetadata + HasRand<R> + HasMaxSize,
    R: Rand,
{
    phantom: PhantomData<(I, R, S)>,
}

impl<I, R, S> Mutator<I, S> for TokenInsert<I, R, S>
where
    I: Input + HasBytesVec,
    S: HasMetadata + HasRand<R> + HasMaxSize,
    R: Rand,
{
    fn mutate(
        &mut self,
        state: &mut S,
        input: &mut I,
        _stage_idx: i32,
    ) -> Result<MutationResult, Error> {
        let max_size = state.max_size();
        let tokens_len = {
            let meta = state.metadata().get::<Tokens>();
            if meta.is_none() {
                return Ok(MutationResult::Skipped);
            }
            if meta.unwrap().tokens().is_empty() {
                return Ok(MutationResult::Skipped);
            }
            meta.unwrap().tokens().len()
        };
        let token_idx = state.rand_mut().below(tokens_len as u64) as usize;

        let size = input.bytes().len();
        let off = state.rand_mut().below((size + 1) as u64) as usize;

        let meta = state.metadata().get::<Tokens>().unwrap();
        let token = &meta.tokens()[token_idx];
        let mut len = token.len();

        if size + len > max_size {
            if max_size > size {
                len = max_size - size;
            } else {
                return Ok(MutationResult::Skipped);
            }
        }

        input.bytes_mut().resize(size + len, 0);
        buffer_self_copy(input.bytes_mut(), off, off + len, size - off);
        buffer_copy(input.bytes_mut(), token, 0, off, len);

        Ok(MutationResult::Mutated)
    }
}

impl<I, R, S> Named for TokenInsert<I, R, S>
where
    I: Input + HasBytesVec,
    S: HasMetadata + HasRand<R> + HasMaxSize,
    R: Rand,
{
    fn name(&self) -> &str {
        "TokenInsert"
    }
}

impl<I, R, S> TokenInsert<I, R, S>
where
    I: Input + HasBytesVec,
    S: HasMetadata + HasRand<R> + HasMaxSize,
    R: Rand,
{
    /// Create a `TokenInsert` `Mutation`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            phantom: PhantomData,
        }
    }
}

/// A `TokenReplace` [`Mutator`] replaces a random part of the input with one of a range of tokens.
/// From AFL terms, this is called as `Dictionary` mutation (which doesn't really make sense ;) ).
#[derive(Debug, Default)]
pub struct TokenReplace<I, R, S>
where
    I: Input + HasBytesVec,
    S: HasMetadata + HasRand<R> + HasMaxSize,
    R: Rand,
{
    phantom: PhantomData<(I, R, S)>,
}

impl<I, R, S> Mutator<I, S> for TokenReplace<I, R, S>
where
    I: Input + HasBytesVec,
    S: HasMetadata + HasRand<R> + HasMaxSize,
    R: Rand,
{
    fn mutate(
        &mut self,
        state: &mut S,
        input: &mut I,
        _stage_idx: i32,
    ) -> Result<MutationResult, Error> {
        let size = input.bytes().len();
        if size == 0 {
            return Ok(MutationResult::Skipped);
        }

        let tokens_len = {
            let meta = state.metadata().get::<Tokens>();
            if meta.is_none() {
                return Ok(MutationResult::Skipped);
            }
            if meta.unwrap().tokens().is_empty() {
                return Ok(MutationResult::Skipped);
            }
            meta.unwrap().tokens().len()
        };
        let token_idx = state.rand_mut().below(tokens_len as u64) as usize;

        let off = state.rand_mut().below(size as u64) as usize;

        let meta = state.metadata().get::<Tokens>().unwrap();
        let token = &meta.tokens()[token_idx];
        let mut len = token.len();
        if off + len > size {
            len = size - off;
        }

        buffer_copy(input.bytes_mut(), token, 0, off, len);

        Ok(MutationResult::Mutated)
    }
}

impl<I, R, S> Named for TokenReplace<I, R, S>
where
    I: Input + HasBytesVec,
    S: HasMetadata + HasRand<R> + HasMaxSize,
    R: Rand,
{
    fn name(&self) -> &str {
        "TokenReplace"
    }
}

impl<I, R, S> TokenReplace<I, R, S>
where
    I: Input + HasBytesVec,
    S: HasMetadata + HasRand<R> + HasMaxSize,
    R: Rand,
{
    /// Creates a new `TokenReplace` struct.
    #[must_use]
    pub fn new() -> Self {
        Self {
            phantom: PhantomData,
        }
    }
}

/// A `I2SRandReplace` [`Mutator`] replaces a random matching input-2-state comparison operand with the other.
/// it needs a valid [`CmpValuesMetadata`] in the state.
#[derive(Debug, Default)]
pub struct I2SRandReplace<I, R, S>
where
    I: Input + HasBytesVec,
    S: HasMetadata + HasRand<R> + HasMaxSize,
    R: Rand,
{
    phantom: PhantomData<(I, R, S)>,
}

impl<I, R, S> Mutator<I, S> for I2SRandReplace<I, R, S>
where
    I: Input + HasBytesVec,
    S: HasMetadata + HasRand<R> + HasMaxSize,
    R: Rand,
{
    #[allow(clippy::too_many_lines)]
    fn mutate(
        &mut self,
        state: &mut S,
        input: &mut I,
        _stage_idx: i32,
    ) -> Result<MutationResult, Error> {
        let size = input.bytes().len();
        if size == 0 {
            return Ok(MutationResult::Skipped);
        }

        let cmps_len = {
            let meta = state.metadata().get::<CmpValuesMetadata>();
            if meta.is_none() {
                return Ok(MutationResult::Skipped);
            }
            if meta.unwrap().list.is_empty() {
                return Ok(MutationResult::Skipped);
            }
            meta.unwrap().list.len()
        };
        let idx = state.rand_mut().below(cmps_len as u64) as usize;

        let off = state.rand_mut().below(size as u64) as usize;
        let len = input.bytes().len();
        let bytes = input.bytes_mut();

        let meta = state.metadata().get::<CmpValuesMetadata>().unwrap();
        let cmp_values = &meta.list[idx];

        let mut result = MutationResult::Skipped;
        match cmp_values {
            CmpValues::U8(v) => {
                for byte in bytes.iter_mut().take(len).skip(off) {
                    if *byte == v.0 {
                        *byte = v.1;
                        result = MutationResult::Mutated;
                        break;
                    } else if *byte == v.1 {
                        *byte = v.0;
                        result = MutationResult::Mutated;
                        break;
                    }
                }
            }
            CmpValues::U16(v) => {
                if len >= size_of::<u16>() {
                    for i in off..len - (size_of::<u16>() - 1) {
                        let val =
                            u16::from_ne_bytes(bytes[i..i + size_of::<u16>()].try_into().unwrap());
                        if val == v.0 {
                            let new_bytes = v.1.to_ne_bytes();
                            bytes[i..i + size_of::<u16>()].copy_from_slice(&new_bytes);
                            result = MutationResult::Mutated;
                            break;
                        } else if val.swap_bytes() == v.0 {
                            let new_bytes = v.1.swap_bytes().to_ne_bytes();
                            bytes[i..i + size_of::<u16>()].copy_from_slice(&new_bytes);
                            result = MutationResult::Mutated;
                            break;
                        } else if val == v.1 {
                            let new_bytes = v.0.to_ne_bytes();
                            bytes[i..i + size_of::<u16>()].copy_from_slice(&new_bytes);
                            result = MutationResult::Mutated;
                            break;
                        } else if val.swap_bytes() == v.1 {
                            let new_bytes = v.0.swap_bytes().to_ne_bytes();
                            bytes[i..i + size_of::<u16>()].copy_from_slice(&new_bytes);
                            result = MutationResult::Mutated;
                            break;
                        }
                    }
                }
            }
            CmpValues::U32(v) => {
                if len >= size_of::<u32>() {
                    for i in off..len - (size_of::<u32>() - 1) {
                        let val =
                            u32::from_ne_bytes(bytes[i..i + size_of::<u32>()].try_into().unwrap());
                        if val == v.0 {
                            let new_bytes = v.1.to_ne_bytes();
                            bytes[i..i + size_of::<u32>()].copy_from_slice(&new_bytes);
                            result = MutationResult::Mutated;
                            break;
                        } else if val.swap_bytes() == v.0 {
                            let new_bytes = v.1.swap_bytes().to_ne_bytes();
                            bytes[i..i + size_of::<u32>()].copy_from_slice(&new_bytes);
                            result = MutationResult::Mutated;
                            break;
                        } else if val == v.1 {
                            let new_bytes = v.0.to_ne_bytes();
                            bytes[i..i + size_of::<u32>()].copy_from_slice(&new_bytes);
                            result = MutationResult::Mutated;
                            break;
                        } else if val.swap_bytes() == v.1 {
                            let new_bytes = v.0.swap_bytes().to_ne_bytes();
                            bytes[i..i + size_of::<u32>()].copy_from_slice(&new_bytes);
                            result = MutationResult::Mutated;
                            break;
                        }
                    }
                }
            }
            CmpValues::U64(v) => {
                if len >= size_of::<u64>() {
                    for i in off..len - (size_of::<u64>() - 1) {
                        let val =
                            u64::from_ne_bytes(bytes[i..i + size_of::<u64>()].try_into().unwrap());
                        if val == v.0 {
                            let new_bytes = v.1.to_ne_bytes();
                            bytes[i..i + size_of::<u64>()].copy_from_slice(&new_bytes);
                            result = MutationResult::Mutated;
                            break;
                        } else if val.swap_bytes() == v.0 {
                            let new_bytes = v.1.swap_bytes().to_ne_bytes();
                            bytes[i..i + size_of::<u64>()].copy_from_slice(&new_bytes);
                            result = MutationResult::Mutated;
                            break;
                        } else if val == v.1 {
                            let new_bytes = v.0.to_ne_bytes();
                            bytes[i..i + size_of::<u64>()].copy_from_slice(&new_bytes);
                            result = MutationResult::Mutated;
                            break;
                        } else if val.swap_bytes() == v.1 {
                            let new_bytes = v.0.swap_bytes().to_ne_bytes();
                            bytes[i..i + size_of::<u64>()].copy_from_slice(&new_bytes);
                            result = MutationResult::Mutated;
                            break;
                        }
                    }
                }
            }
            CmpValues::Bytes(v) => {
                'outer: for i in off..len {
                    let mut size = core::cmp::min(v.0.len(), len - i);
                    while size != 0 {
                        if v.0[0..size] == input.bytes()[i..i + size] {
                            buffer_copy(input.bytes_mut(), &v.1, 0, i, size);
                            break 'outer;
                        }
                        size -= 1;
                    }
                    size = core::cmp::min(v.1.len(), len - i);
                    while size != 0 {
                        if v.1[0..size] == input.bytes()[i..i + size] {
                            buffer_copy(input.bytes_mut(), &v.0, 0, i, size);
                            break 'outer;
                        }
                        size -= 1;
                    }
                }
            }
        }

        //println!("{:?}", result);

        Ok(result)
    }
}

impl<I, R, S> Named for I2SRandReplace<I, R, S>
where
    I: Input + HasBytesVec,
    S: HasMetadata + HasRand<R> + HasMaxSize,
    R: Rand,
{
    fn name(&self) -> &str {
        "I2SRandReplace"
    }
}

impl<I, R, S> I2SRandReplace<I, R, S>
where
    I: Input + HasBytesVec,
    S: HasMetadata + HasRand<R> + HasMaxSize,
    R: Rand,
{
    /// Creates a new `I2SRandReplace` struct.
    #[must_use]
    pub fn new() -> Self {
        Self {
            phantom: PhantomData,
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "std")]
    use std::fs;

    #[cfg(feature = "std")]
    use super::Tokens;

    #[cfg(feature = "std")]
    #[test]
    fn test_read_tokens() {
        let _res = fs::remove_file("test.tkns");
        let data = r###"
# comment
token1@123="AAA"
token1="A\x41A"
"A\AA"
token2="B"
        "###;
        fs::write("test.tkns", data).expect("Unable to write test.tkns");
        let tokens = Tokens::from_tokens_file(&"test.tkns").unwrap();
        #[cfg(feature = "std")]
        println!("Token file entries: {:?}", tokens.tokens());
        assert_eq!(tokens.tokens().len(), 2);
        let _res = fs::remove_file("test.tkns");
    }
}
