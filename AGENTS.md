# Repository instructions

## Commits

- Follow Conventional Commits for every commit subject.
- Use `<type>(<optional-scope>): <description>`.
- Use one of the repository's allowed types: `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`, `build`, `ci`, `chore`, or `revert`.
- Keep the complete commit header to 72 characters or fewer.
- Before committing, validate the exact final subject against `commitlint.config.mjs`.
- Keep PR titles compatible with the same rules so squash-merge commit titles pass commitlint. For example, use `feat(pi): add Pi session support (#71)`, not `Add Pi session support (#71)`.
