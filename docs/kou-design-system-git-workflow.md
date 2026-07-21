# Kou Design System Git Workflow

`@kou/ui-kit` живёт в отдельном public repo:

https://github.com/gfhfyjbr/kou-design-system

В `kou-router` он подключён как **git submodule** + `file:` dependency:

```
frontend/vendor/kou-ui-kit   # submodule path
```

```json
"@kou/ui-kit": "file:./vendor/kou-ui-kit"
```

Vite aliases `@kou/ui-kit` на `frontend/vendor/kou-ui-kit/src` для HMR.

## Clone kou-router

```sh
git clone --recurse-submodules git@github.com:gfhfyjbr/kou-router.git
cd kou-router
```

Если уже клонировал без submodules:

```sh
git submodule update --init --recursive
```

Frontend:

```sh
cd frontend
bun install
bun run build
```

## Develop UI-kit

Preferred local checkout for design-system work:

```sh
# sibling checkout
git clone git@github.com:gfhfyjbr/kou-design-system.git ~/Projects/JS/kou-ui-kit
```

Or edit the submodule in place:

```sh
cd frontend/vendor/kou-ui-kit
bun install
bun run typecheck
bun run build
```

Commit inside the submodule, push design-system, then in kou-router commit the updated submodule pointer:

```sh
cd frontend/vendor/kou-ui-kit
git add -A && git commit -m "feat: ..." && git push
cd ../../..
git add frontend/vendor/kou-ui-kit
git commit -m "chore(frontend): bump kou-ui-kit submodule"
```

## Temporary override

If the submodule is not yet pointing at a pushed commit, the in-tree
`frontend/vendor/kou-ui-kit` package still works via `file:./vendor/kou-ui-kit`.
Do not commit absolute local paths in `frontend/package.json`.
