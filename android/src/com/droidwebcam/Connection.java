package com.droidwebcam;

import java.io.DataInputStream;
import java.io.DataOutputStream;
import java.io.IOException;
import java.net.InetAddress;
import java.net.InetSocketAddress;
import java.net.Socket;
import java.security.GeneralSecurityException;
import java.security.MessageDigest;
import java.security.cert.CertificateException;
import java.security.cert.X509Certificate;
import java.util.Arrays;
import javax.net.ssl.SSLContext;
import javax.net.ssl.SSLSocket;
import javax.net.ssl.TrustManager;
import javax.net.ssl.X509TrustManager;

final class Connection implements AutoCloseable {
    static final int MAX_JPEG = 3 * 1024 * 1024;
    private static final int LEGACY_MAX_JPEG = 256 * 1024;
    private static final int PAIR_TIMEOUT_MS = 8000;
    private static final int FRAME_TIMEOUT_MS = 3000;
    private static final int PAIR_PROOF = 0x44573032; // DW02
    private static final int LEGACY_PROOF = 0x44573031;
    private static final int PAIRED = 0x44574f4b; // DWOK
    private final int maxJpeg;
    private final SSLSocket tls;
    private final DataInputStream in;
    private final DataOutputStream out;

    static final class Invite {
        final int version;
        final InetSocketAddress address;
        final byte[] pin;
        final byte[] secret;

        private Invite(int version, InetSocketAddress address, byte[] pin, byte[] secret) {
            this.version = version;
            this.address = address;
            this.pin = pin;
            this.secret = secret;
        }

        static Invite parse(String code) throws IOException {
            String[] parts = code.trim().split(":", -1);
            if (parts.length != 6 || !parts[0].equals("droidcam") || !(parts[1].equals("1") || parts[1].equals("2"))) {
                throw new IOException("Invalid invitation. Copy the entire code from the host.");
            }
            String[] ip = parts[2].split("\\.", -1);
            if (ip.length != 4) throw new IOException("Expected a host IPv4 address.");
            byte[] address = new byte[4];
            for (int i = 0; i < ip.length; i++) {
                address[i] = (byte) parseDecimal(ip[i], 255);
            }
            InetAddress host = InetAddress.getByAddress(address);
            int port = parseDecimal(parts[3], 65535);
            if (port == 0 || host.isAnyLocalAddress() || host.isMulticastAddress()
                    || Arrays.equals(address, new byte[] {-1, -1, -1, -1})) {
                throw new IOException("Expected a unicast host address and port.");
            }
            return new Invite(Integer.parseInt(parts[1]), new InetSocketAddress(host, port), hex(parts[4]), hex(parts[5]));
        }

        private static int parseDecimal(String value, int max) throws IOException {
            if (!value.matches("0|[1-9][0-9]{0,4}")) throw new IOException("Invalid host address or port.");
            int number = Integer.parseInt(value);
            if (number > max) throw new IOException("Invalid host address or port.");
            return number;
        }

        private static byte[] hex(String value) throws IOException {
            if (!value.matches("[0-9a-fA-F]{64}")) throw new IOException("Invalid invitation key.");
            byte[] bytes = new byte[32];
            for (int i = 0; i < bytes.length; i++) {
                bytes[i] = (byte) Integer.parseInt(value.substring(i * 2, i * 2 + 2), 16);
            }
            return bytes;
        }
    }

    static Connection open(Socket socket, Invite invite) throws IOException, GeneralSecurityException {
        return open(socket, invite, invite.version == 1 ? CaptureSettings.legacy() : CaptureSettings.defaults());
    }

    static Connection open(Socket socket, Invite invite, CaptureSettings settings) throws IOException, GeneralSecurityException {
        if (invite.version == 1 && !settings.matches(CaptureSettings.legacy().wire())) {
            throw new IOException("Update the host to choose capture settings.");
        }
        socket.connect(invite.address, PAIR_TIMEOUT_MS);
        socket.setTcpNoDelay(true);
        socket.setSoTimeout(PAIR_TIMEOUT_MS);
        SSLContext context = SSLContext.getInstance("TLSv1.3");
        context.init(null, new TrustManager[] { new PinnedCertificate(invite.pin) }, null);
        SSLSocket tls = (SSLSocket) context.getSocketFactory().createSocket(
                socket, "droid-webcam.local", invite.address.getPort(), true);
        boolean connected = false;
        try {
            tls.setSoTimeout(PAIR_TIMEOUT_MS);
            tls.setEnabledProtocols(new String[] { "TLSv1.3" });
            tls.startHandshake();
            Connection connection = new Connection(tls, invite.version == 1 ? LEGACY_MAX_JPEG : MAX_JPEG);
            connection.out.writeInt(invite.version == 1 ? LEGACY_PROOF : PAIR_PROOF);
            connection.out.write(invite.secret);
            if (invite.version == 2) connection.out.write(settings.wire());
            connection.out.flush();
            if (connection.in.readInt() != PAIRED) throw new IOException("Host rejected pairing.");
            if (invite.version == 2) {
                byte[] accepted = new byte[8];
                connection.in.readFully(accepted);
                if (!settings.matches(accepted)) throw new IOException("Host changed the capture settings.");
            }
            tls.setSoTimeout(FRAME_TIMEOUT_MS);
            connected = true;
            return connection;
        } finally {
            if (!connected) tls.close();
        }
    }

    private Connection(SSLSocket tls, int maxJpeg) throws IOException {
        this.maxJpeg = maxJpeg;
        this.tls = tls;
        this.in = new DataInputStream(tls.getInputStream());
        this.out = new DataOutputStream(tls.getOutputStream());
    }

    void send(byte[] jpeg) throws IOException {
        if (jpeg.length == 0 || jpeg.length > maxJpeg) throw new IOException("Invalid JPEG size.");
        out.writeInt(jpeg.length);
        out.write(jpeg);
        out.flush();
        if (in.readUnsignedByte() != 1) throw new IOException("Invalid frame acknowledgement.");
    }

    void stop() throws IOException {
        out.writeInt(0);
        out.flush();
    }

    @Override public void close() throws IOException { tls.close(); }

    private static final class PinnedCertificate implements X509TrustManager {
        private final byte[] pin;

        PinnedCertificate(byte[] pin) { this.pin = pin.clone(); }

        @Override public void checkServerTrusted(X509Certificate[] chain, String authType)
                throws CertificateException {
            if (chain == null || chain.length == 0) throw new CertificateException("Host has no certificate.");
            try {
                byte[] fingerprint = MessageDigest.getInstance("SHA-256").digest(chain[0].getEncoded());
                if (!MessageDigest.isEqual(pin, fingerprint)) {
                    throw new CertificateException("Host fingerprint does not match the invitation.");
                }
            } catch (GeneralSecurityException error) {
                throw new CertificateException(error.getMessage(), error);
            }
        }

        @Override public void checkClientTrusted(X509Certificate[] chain, String authType)
                throws CertificateException {
            throw new CertificateException("Client trust is unavailable.");
        }

        @Override public X509Certificate[] getAcceptedIssuers() { return new X509Certificate[0]; }
    }
}
