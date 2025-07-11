FROM rust:1.88.0 AS builder

WORKDIR /app

# Install wasm-pack
RUN cargo install wasm-pack

# Copy necessary files for building
COPY Cargo.toml .
COPY lib lib
COPY node node
COPY webclient webclient

# Build the webclient
RUN wasm-pack build webclient --target web --release

# Stage 2: Serve with Nginx
FROM nginx:1.27.0-alpine

# Remove default Nginx configuration
RUN rm /etc/nginx/conf.d/default.conf

# Copy custom Nginx configuration
COPY nginx.conf /etc/nginx/conf.d/default.conf

# Copy built webclient and all public assets
COPY --from=builder /app/webclient/pkg /usr/share/nginx/html/webclient/pkg
COPY public /usr/share/nginx/html

EXPOSE 80

CMD ["nginx", "-g", "daemon off;"]