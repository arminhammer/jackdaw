# compute-hashes

Compute SHA256 hashes of file categories for CI cache invalidation.

## Version

1.0.0

## Description

This function computes deterministic SHA256 hashes for different categories of files in the repository. The hashes are used as cache keys to determine whether CI tasks need to be re-executed.

## Input Schema

```yaml
categories:
  type: object
  description: Map of category name to file patterns
  additionalProperties:
    type: object
    properties:
      patterns:
        type: array
        items:
          type: string
        description: Glob patterns to match files
    required:
      - patterns
```

## Output Schema

```json
{
  "results": {
    "category_name": {
      "hash": "sha256_hash_string",
      "files": ["file1.rs", "file2.rs"],
      "count": 2
    }
  }
}
```

## Usage Example

```yaml
- computeHashes:
    call: compute-hashes
    with:
      categories:
        rust_sources:
          patterns:
            - 'src/**/*.rs'
            - 'Cargo.toml'
            - 'Cargo.lock'
    output:
      as: .hashes
```

## Implementation

- Language: Python 3.10+
- Type-safe: Uses `TypedDict` for input/output schemas
- Deterministic: Files are sorted before hashing for consistent results
- Error handling: Gracefully skips files that can't be read
