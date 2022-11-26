// Copyright 2022 Google LLC
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::{
    fs::File,
    io::{BufReader, Read, Seek, SeekFrom},
    sync::{Arc, Mutex},
};

/// A trait to represent some reader which has a total length known in
/// advance. This is roughly equivalent to the nightly
/// [`Seek::stream_len`] API.
pub(crate) trait HasLength {
    /// Return the current total length of this stream.
    fn len(&self) -> u64;
}

/// A [`Read`] which refers to its underlying stream by reference count,
/// and thus can be cloned cheaply. It supports seeking; each cloned instance
/// maintains its own pointer into the file, and the underlying instance
/// is seeked prior to each read.
pub(crate) struct CloneableSeekableReader<R: Read + Seek + HasLength> {
    file: Arc<Mutex<R>>,
    pos: u64,
    // TODO determine and store this once instead of per cloneable file
    file_length: Option<u64>,
}

impl<R: Read + Seek + HasLength> Clone for CloneableSeekableReader<R> {
    fn clone(&self) -> Self {
        Self {
            file: self.file.clone(),
            pos: self.pos,
            file_length: self.file_length,
        }
    }
}

impl<R: Read + Seek + HasLength> CloneableSeekableReader<R> {
    /// Constructor. Takes ownership of the underlying `Read`.
    /// You should pass in only streams whose total length you expect
    /// to be fixed and unchanging. Odd behavior may occur if the length
    /// of the stream changes; any subsequent seeks will not take account
    /// of the changed stream length.
    pub(crate) fn new(file: R) -> Self {
        Self {
            file: Arc::new(Mutex::new(file)),
            pos: 0u64,
            file_length: None,
        }
    }

    /// Determine the length of the underlying stream.
    fn ascertain_file_length(&mut self) -> u64 {
        match self.file_length {
            Some(file_length) => file_length,
            None => {
                let len = self.file.lock().unwrap().len();
                self.file_length = Some(len);
                len
            }
        }
    }
}

impl<R: Read + Seek + HasLength> Read for CloneableSeekableReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut underlying_file = self.file.lock().expect("Unable to get underlying file");
        // TODO share an object which knows current position to avoid unnecessary
        // seeks
        underlying_file.seek(SeekFrom::Start(self.pos))?;
        let read_result = underlying_file.read(buf);
        if let Ok(bytes_read) = read_result {
            // TODO, once stabilised, use checked_add_signed
            self.pos += bytes_read as u64;
        }
        read_result
    }
}

impl<R: Read + Seek + HasLength> Seek for CloneableSeekableReader<R> {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let new_pos = match pos {
            SeekFrom::Start(pos) => pos,
            SeekFrom::End(offset_from_end) => {
                let file_len = self.ascertain_file_length();
                // TODO, once stabilised, use checked_add_signed
                file_len - (-offset_from_end as u64)
            }
            // TODO, once stabilised, use checked_add_signed
            SeekFrom::Current(offset_from_pos) => {
                if offset_from_pos > 0 {
                    self.pos + (offset_from_pos as u64)
                } else {
                    self.pos - ((-offset_from_pos) as u64)
                }
            }
        };
        self.pos = new_pos;
        Ok(new_pos)
    }
}

impl<R: HasLength> HasLength for BufReader<R> {
    fn len(&self) -> u64 {
        self.get_ref().len()
    }
}

impl HasLength for File {
    fn len(&self) -> u64 {
        self.metadata().unwrap().len()
    }
}

#[cfg(test)]
mod test {
    use super::{CloneableSeekableReader, HasLength};
    use std::io::{Cursor, Read, Seek, SeekFrom};
    use test_log::test;

    impl HasLength for Cursor<Vec<u8>> {
        fn len(&self) -> u64 {
            self.get_ref().len() as u64
        }
    }

    #[test]
    fn test_cloneable_seekable_reader() {
        let buf: Vec<u8> = vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9];
        let buf = Cursor::new(buf);
        let mut reader = CloneableSeekableReader::new(buf);
        let mut out = vec![0; 2];
        assert!(reader.read_exact(&mut out).is_ok());
        assert_eq!(out[0], 0);
        assert_eq!(out[1], 1);
        assert!(reader.seek(SeekFrom::Start(0)).is_ok());
        assert!(reader.read_exact(&mut out).is_ok());
        assert_eq!(out[0], 0);
        assert_eq!(out[1], 1);
        assert!(reader.seek(SeekFrom::Current(0)).is_ok());
        assert!(reader.read_exact(&mut out).is_ok());
        assert_eq!(out[0], 2);
        assert_eq!(out[1], 3);
        assert!(reader.seek(SeekFrom::End(-2)).is_ok());
        assert!(reader.read_exact(&mut out).is_ok());
        assert_eq!(out[0], 8);
        assert_eq!(out[1], 9);
        assert!(reader.read_exact(&mut out).is_err());
    }
}
