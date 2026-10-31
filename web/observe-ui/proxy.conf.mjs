// `npm start` proxies the API to a running `icers --observe` dashboard.
// ICERS_OBSERVE_URL defaults to http://127.0.0.1:4659; ICERS_OBSERVE_TOKEN is
// the token from the printed dashboard URL (also ~/.icers/agent/observe.token).
const target = process.env.ICERS_OBSERVE_URL ?? 'http://127.0.0.1:4659';
const token = process.env.ICERS_OBSERVE_TOKEN ?? '';

export default ['/api', '/state', '/runs', '/replay', '/events'].reduce((config, path) => {
  config[path] = {
    target,
    changeOrigin: false,
    headers: { Authorization: `Bearer ${token}`, Host: '127.0.0.1' },
  };
  return config;
}, {});
