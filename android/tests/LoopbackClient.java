package com.droidwebcam;

import java.awt.image.BufferedImage;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.net.Socket;
import java.security.GeneralSecurityException;
import java.util.Random;
import javax.imageio.IIOImage;
import javax.imageio.ImageIO;
import javax.imageio.ImageWriteParam;
import javax.imageio.ImageWriter;
import javax.imageio.stream.ImageOutputStream;

public final class LoopbackClient {
    public static void main(String[] args) throws Exception {
        if (!CaptureSettings.supportsFrameDuration(33_333_334L, 30)
                || !CaptureSettings.supportsFrameDuration(16_666_667L, 60)
                || !CaptureSettings.supportsFrameDuration(0, 60)
                || CaptureSettings.supportsFrameDuration(33_333_334L, 60)
                || CaptureSettings.supportsFrameDuration(1, 0)) {
            throw new AssertionError("Frame duration filtering failed");
        }
        String code = new java.io.BufferedReader(new java.io.InputStreamReader(System.in)).readLine();
        boolean legacy = args.length == 1 && args[0].equals("legacy");
        CaptureSettings settings = legacy ? CaptureSettings.legacy() : new CaptureSettings(
                Integer.parseInt(args[0]), Integer.parseInt(args[1]), Integer.parseInt(args[2]),
                Integer.parseInt(args[3]), Boolean.parseBoolean(args[4]));
        if (legacy) code = code.replace("droidcam:2:", "droidcam:1:");
        Connection.Invite invite = Connection.Invite.parse(code);
        invite.pin[0] ^= 1;
        expectRejected(invite, settings);
        invite.pin[0] ^= 1;
        invite.secret[0] ^= 1;
        expectRejected(invite, settings);
        invite.secret[0] ^= 1;
        try (Socket socket = new Socket(); Connection connection = Connection.open(socket, invite, settings)) {
            BufferedImage image = new BufferedImage(settings.width, settings.height, BufferedImage.TYPE_INT_RGB);
            int[] noise = new int[settings.width * settings.height];
            Random random = new Random(42);
            for (int i = 0; i < noise.length; i++) noise[i] = random.nextInt(0x1000000);
            image.setRGB(0, 0, settings.width, settings.height, noise, 0, settings.width);
            ByteArrayOutputStream jpeg = new ByteArrayOutputStream();
            ImageWriter writer = ImageIO.getImageWritersByFormatName("jpeg").next();
            try (ImageOutputStream output = ImageIO.createImageOutputStream(jpeg)) {
                writer.setOutput(output);
                ImageWriteParam options = writer.getDefaultWriteParam();
                options.setCompressionMode(ImageWriteParam.MODE_EXPLICIT);
                options.setCompressionQuality(settings.quality / 100.0f);
                writer.write(null, new IIOImage(image, null, null), options);
            } finally { writer.dispose(); }
            if (!legacy && settings.width >= 1280 && jpeg.size() <= 256 * 1024) {
                throw new AssertionError("Test JPEG did not exercise the larger frame limit");
            }
            connection.send(jpeg.toByteArray());
            connection.stop();
        }
    }

    private static void expectRejected(Connection.Invite invite, CaptureSettings settings) throws Exception {
        try (Socket socket = new Socket(); Connection ignored = Connection.open(socket, invite, settings)) {
            throw new AssertionError("Invalid credentials were accepted");
        } catch (IOException | GeneralSecurityException expected) {
            // An unauthenticated attempt must leave the one-time invitation usable.
        }
    }
}
