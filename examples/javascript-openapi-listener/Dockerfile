FROM jackdaw:latest

WORKDIR /app

# Copy JavaScript handler files
# Note: Jackdaw uses embedded Deno runtime (rustyscript) - no need to install Deno
COPY src ./src
COPY deno.json ./
COPY openapi.spec.yaml ./
COPY calculator-api.sw.yaml ./

# Expose HTTP port
EXPOSE 8080

# Note: Base image sets ENTRYPOINT to jackdaw, so we just pass the args
CMD ["run", "calculator-api.sw.yaml", "--debug"]
