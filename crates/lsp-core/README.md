# lsp-core

The parts of the servers that do not depend on the language: stdio transport,
main loop, requests to the client, diagnostic timers, `runSafe` and the
settings of each document. Responses and notifications are serialized directly
from typed values.
