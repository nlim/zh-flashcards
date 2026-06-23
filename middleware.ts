export const config = {
  matcher: [
    // All non-static paths (covers / and any future pages)
    '/((?!_vercel|.*\\..*).*)',
    // All API routes
    '/api/:path*',
  ],
};

export default function middleware(request: Request): Response | void {
  const auth = request.headers.get('authorization');

  const expectedUser = process.env.AUTH_USER ?? 'admin';
  const expectedPass = process.env.AUTH_PASS ?? 'password';

  const deny = (msg: string) =>
    new Response(msg, {
      status: 401,
      headers: { 'WWW-Authenticate': 'Basic realm="ZhongwenFlashcards", charset="UTF-8"' },
    });

  if (!auth?.startsWith('Basic ')) {
    return deny('Authentication required');
  }

  let decoded: string;
  try {
    decoded = atob(auth.slice(6).trim());
  } catch {
    return deny('Unauthorized');
  }

  // Split only on the first colon so passwords containing colons work
  const colonIdx = decoded.indexOf(':');
  const user = colonIdx >= 0 ? decoded.slice(0, colonIdx) : decoded;
  const pass = colonIdx >= 0 ? decoded.slice(colonIdx + 1) : '';

  if (user !== expectedUser || pass !== expectedPass) {
    return deny('Unauthorized');
  }

  // Authorized — fall through to origin (static file or Rust function)
}

