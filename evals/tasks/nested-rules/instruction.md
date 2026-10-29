Add a public function `percent(part: u64, whole: u64) -> Option<u8>` to the app package: the
share `part` is of `whole`, as a whole percent rounded down, or `None` when `whole` is 0 or
`part` is more than `whole`. Add tests for it.
