# Cursor CLI ACP

Jcode can use Cursor's native Agent Client Protocol transport as a separate provider:

```sh
jcode --provider cursor-acp
```

The provider launches `agent acp` in the current working directory. Cursor CLI owns
authentication, tools, permissions, and model availability. Authenticate Cursor
outside jcode with the Cursor CLI login flow.

## Controlled executable

Use environment variables when the Cursor executable is not named `agent`:

```sh
export JCODE_CURSOR_ACP_PATH=/path/to/agent
export JCODE_CURSOR_ACP_ARGS='acp'
# Optional exact advertised model ID.
export JCODE_CURSOR_ACP_MODEL='composer-2.5[fast=true]'
```

`JCODE_CURSOR_ACP_ARGS` is passed as direct arguments, never through a shell.
`JCODE_CURSOR_ACP_MODEL` is optional and is validated against the ACP-advertised
catalog before use.

Permission requests are denied unless an advertised option matches the configured
value. To select a Cursor ACP permission option explicitly:

```sh
export JCODE_CURSOR_ACP_PERMISSION=allow_once
```

The default is `reject_once`. This keeps a missing or changed Cursor permission
schema fail-closed.

## Model discovery and resolution

Jcode does not maintain a static Cursor ACP model list and does not call Cursor's
direct HTTP model endpoint. It reads the model catalog from `session/new` and
subsequent ACP configuration updates.

Model selection rules:

1. No requested model uses Cursor's advertised current model.
2. An exact advertised ID is selected unchanged.
3. A bare ID is accepted only when exactly one advertised variant has that base.
4. Unsupported or ambiguous IDs return an explicit error.

Bracketed settings are opaque and preserved, for example
`gpt-5.6-sol[context=272k,reasoning=medium,fast=false]`.

The ACP route is intentionally separate from `--provider cursor`, which remains
the direct Cursor HTTPS provider.
