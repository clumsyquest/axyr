# HTTP API

The engine exposes a small HTTP API alongside the MCP stdio interface. It is a
second **front-end** over the same dispatch as MCP, so the web dashboard sees
exactly what an MCP agent sees — the engine never has two different views of the
system.

- **Address:** `127.0.0.1:7878` by default. Set `AXYR_HTTP=<addr:port>` to change
  it, or `AXYR_HTTP=off` to disable the API (MCP stdio still works).
- **CORS:** permissive (`Access-Control-Allow-Origin: *`), so a dashboard served
  from another origin can fetch it directly.
- **Content type:** JSON tools return `application/json`; the rest return
  `text/plain`. Errors return the message as plain text with a `4xx` status.

The live index (`GET /`) returns this same list as JSON.

## Reads (GET)

| Route | Returns |
| --- | --- |
| `/` | this endpoint list (JSON) |
| `/snapshot` | the whole system snapshot — the data contract (JSON) |
| `/graph` | the system as `{board, nodes, edges, disabled}` for the schematic view (JSON) |
| `/system_map` | the hardware map (text) |
| `/threads` | live RTOS thread state (text) |
| `/trace` | context-switch timeline (text) |
| `/history` | recorded time-series of system state, for animation (JSON) |
| `/health` | proactive anomaly checks (text) |
| `/crash` | the last captured crash (text) |
| `/variables` | global variables discovered from the ELF (text) |
| `/peripherals` | chip peripherals discovered from the SVD (text) |
| `/firmware` | flashable images found on the host (text) |
| `/variable?name=<sym>` | read one global live |
| `/peripheral?name=<NAME>` | decode one peripheral's live registers |
| `/memory?address=<hex>&count=<n>` | read `n` 32-bit words |

## Actions (POST)

| Route | Body | Effect |
| --- | --- | --- |
| `/reboot` | — | reset the board and let it run |
| `/diff` | — | snapshot diff vs the previous snapshot |
| `/flash` | `{"path": "<elf>"}` | flash an ELF image, then run |
| `/watch` | `{"name": "<sym>", "value": <n>}` | non-intrusive wait for a value |

## Graph shape (`/graph`)

```json
{
  "board": "STMicroelectronics STM32F401RE-NUCLEO board",
  "nodes": [{ "id": "i2c1 arduino_i2c", "kind": "i2c bus", "address": "0x40005400", "status": "okay" }],
  "edges": [{ "from": "soc", "to": "i2c1 arduino_i2c" }],
  "disabled": ["i2c2", "spi3", "..."]
}
```

A synthetic `board` root connects the top-level devices (CPU, RAM, clocks,
sensors, headers); each device links to its parent, so a sensor on a bus reads as
`soc -> i2c1 -> bme280`. Live values/activity are layered on top from
`/snapshot` and `/history`.
