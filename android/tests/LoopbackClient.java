package com.droidwebcam;

import java.awt.Color;
import java.awt.image.BufferedImage;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.net.Socket;
import java.security.GeneralSecurityException;
import javax.imageio.ImageIO;

public final class LoopbackClient {
    public static void main(String[] args) throws Exception {
        Connection.Invite invite = Connection.Invite.parse(
                new java.io.BufferedReader(new java.io.InputStreamReader(System.in)).readLine());
        invite.pin[0] ^= 1;
        expectRejected(invite);
        invite.pin[0] ^= 1;
        invite.secret[0] ^= 1;
        expectRejected(invite);
        invite.secret[0] ^= 1;
        try (Socket socket = new Socket(); Connection connection = Connection.open(socket, invite)) {
            BufferedImage image = new BufferedImage(640, 480, BufferedImage.TYPE_INT_RGB);
            image.setRGB(80, 80, Color.CYAN.getRGB());
            ByteArrayOutputStream jpeg = new ByteArrayOutputStream();
            if (!ImageIO.write(image, "jpg", jpeg)) throw new AssertionError("Missing JPEG writer");
            connection.send(jpeg.toByteArray());
            connection.stop();
        }
    }

    private static void expectRejected(Connection.Invite invite) throws Exception {
        try (Socket socket = new Socket(); Connection ignored = Connection.open(socket, invite)) {
            throw new AssertionError("Invalid credentials were accepted");
        } catch (IOException | GeneralSecurityException expected) {
            // Continue on the same invitation, which must still be valid.
        }
    }
}
