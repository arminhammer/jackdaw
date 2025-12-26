FROM jackdaw:latest

WORKDIR /app

# Copy JavaScript handler files
# Note: Jackdaw uses embedded Deno runtime (rustyscript) - no need to install Deno
COPY src ./src
COPY deno.json ./
COPY spec.proto ./
COPY calculator-api.sw.yaml ./

# Expose gRPC port
EXPOSE 50051

# Note: Base image sets ENTRYPOINT to jackdaw, so we just pass the args
CMD ["run", "calculator-api.sw.yaml", "--debug"]
