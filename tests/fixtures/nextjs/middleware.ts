// Edge middleware runs before every route; it is not an endpoint.
export function middleware() {}
export const config = { matcher: "/api/:path*" };
