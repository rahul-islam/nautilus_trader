# Groww feed protobuf schemas

Groww streams market data, order updates, and position updates as protobuf messages over NATS.
The venue does not publish `.proto` sources; it ships only the generated Python modules inside the
`growwapi` package. The definitions here were recovered from the `FileDescriptorProto` embedded in
those modules, so field numbers and wire types match what the venue emits. Field *names* were
converted to `snake_case`, which does not affect the binary wire format.

The Rust types generated from these schemas are checked in under `src/websocket/proto/`. They are
vendored rather than generated at build time so that building the workspace does not require a
`protoc` binary.

## Regenerating

Only needed when Groww changes a schema.

1. Recover the current descriptors from the installed `growwapi` package and update the `.proto`
   files here to match.
2. Compile the schemas to a descriptor set (any protobuf compiler will do; `grpcio-tools` ships
   one and needs no system install):

   ```bash
   python -m grpc_tools.protoc -I proto --include_imports \
       --descriptor_set_out=/tmp/groww.fds \
       proto/stocks_socket_response.proto \
       proto/stock_orders_socket_response.proto \
       proto/position_socket.proto
   ```

3. Run `prost-build` over the descriptor set and copy the output into `src/websocket/proto/`,
   keeping the existing module headers.
