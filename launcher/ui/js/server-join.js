// Keep these rules in step with server_join_address in omsi-launcher-model.
export function serverJoinSettings(address, protocol = 'auto', info) {
  address = address.trim();
  const web = /^(https?|wss?):\/\//.test(address)
    || address.toLowerCase() === 'openomsi'
    || address.endsWith('.trycloudflare.com') || address.includes('.trycloudflare.com/');
  const port = address.match(/^(?:[^:\s/]+|\[[^\]\s]+\]):([0-9]+)$/)?.[1];
  const udp = protocol === 'udp' || (protocol === 'auto' && !web && port != null && Number(port) <= 65535);
  if (udp && web) throw new Error("UDP needs the game's address (host or host:port), not a web link");
  if (!info) throw new Error('The server has not answered yet (is its address right? is it running?)');
  return {
    lan_addr: !udp && !web && info.reached_at ? info.reached_at : address,
    lan_ws_fallback: protocol === 'auto' && udp ? info.reached_at || null : null,
  };
}
