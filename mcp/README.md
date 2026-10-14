# @blockula/mcp

🧛 MCP server that lets any coding agent use **Blockula**, a post-quantum blockchain. Agents can hold a wallet, send FANG, claim `.boo` names, look things up in plain English, and start or summon chains.

```bash
claude mcp add blockula -- npx -y @blockula/mcp
```

**Tools:** `status`, `start_dev_node`, `summon_chain`, `create_wallet`, `use_account`, `wallet_info`, `faucet`, `send`, `check_name`, `register_name`, `lookup`, `latest_blocks`.

**Spending a user's money safely:**
1. `create_wallet` gives you a `key_id`. Hand it to the user.
2. The user grants it a budget in the Blockula web wallet (Devices & agents → Give an agent a budget). The chain enforces the limit and expiry.
3. `use_account("theirname.boo")`, then `send`.

| Env | Default | |
|---|---|---|
| `BLOCKULA_RPC` | `http://localhost:7070` | node URL |
| `BLOCKULA_HOME` | `~/.blockula` | where the agent key is stored (mode 600) |
| `BLOCKULA_MAX_SEND` | `100` | max FANG per `send` call |
| `BLOCKULA_BIN` | `blockula` | node binary used by `start_dev_node` / `summon_chain` |

`start_dev_node` and `summon_chain` need the `blockula` node binary on your PATH. All other tools work against any reachable node.

⚠️ Early and unaudited: test chains only.
