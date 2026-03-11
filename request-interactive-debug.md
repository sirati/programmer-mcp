please use debug-mcp to debug the programmer-mcp, you should also try to relay commands through it, and see how the responses are.

in general the response should be done in a way that does not include status messages, if operations that change stuff succeed, all that should be returned is: "X change operations succeeded" only when they fail should the failure reason itself be reported.

operations that are to aquire data should be combined, e.g. the response of all inspected symbols should together. such opperation should only return the content, no status messages that report success. if alternative names were selected this should be minimally made clear e.g. logServer -> LogServer: {response}, if suggestions are generated they should be reported. if an operations fails in other ways no failure should be reported, but the response should be postfixed with a single message akin to "Some requests found nothing"
